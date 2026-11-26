# Batch 8 - Agent M6: Weather Provider (Milestone 6)

**Date:** 2026-11-26
**Baseline:** HEAD `2bd2513`, all existing uncommitted Batch 3-7 work preserved.
**Scope:** Live Open-Meteo weather provider for the M2 `WeatherProvider` boundary.

---

## 1. Status

Implemented and verified.

- No commits, deploys, resets, rebases, or cleanup commands were run.
- No files outside the M6 allowlist were modified.
- No new dependencies were added (`std::process::Command` + existing `serde_json`).
- No API keys or secrets are used or stored. Open-Meteo requires no key.
- Provider failures are typed `WeatherUnavailable` values and do not break the
  Platform snapshot pipeline.
- Stale cached data is never presented as current: cache hits are only used
  within the TTL, and expired entries are refetched.

### Files

| File | Change |
|---|---|
| `crates/aegis-platform/src/weather.rs` | New `OpenMeteoProvider`, strict parser, WMO mapping, bounded TTL cache, unit tests. |
| `crates/aegis-platform/src/lib.rs` | Added `mod weather;` and public re-exports only. |
| `crates/aegis-platform/tests/weather.rs` | New integration tests and opt-in live network test. |
| `docs/implementation/BATCH8_AGENT06_M6_WEATHER.md` | This document. |

---

## 2. Provider and API

`OpenMeteoProvider` implements the existing M2 trait:

```rust
pub trait WeatherProvider {
    fn name(&self) -> &'static str; // "open-meteo"
    fn observe(&self, request: &WeatherRequest)
        -> Result<WeatherData, WeatherUnavailable>;
}
```

Request used:

- `GET https://api.open-meteo.com/v1/forecast`
- `lat`/`lon` come from `WeatherRequest`; both are required.
- `timezone` comes from `WeatherRequest` (IANA, already validated by M2 config).
- `forecast_days=3`
- `current`: `temperature_2m`, `apparent_temperature`,
  `relative_humidity_2m`, `precipitation`, `precipitation_probability`,
  `weather_code`, `wind_speed_10m`, `is_day`
- `daily`: `temperature_2m_min`, `temperature_2m_max`,
  `precipitation_probability_max`, `weather_code`, `sunrise`, `sunset`

Transport is `curl.exe` through `std::process::Command` with an explicit
argument vector, never a shell string:

- `-sS`, `--fail`, `-m 15` (`--max-time 15`)
- `--max-filesize 65536` plus a 64 KiB post-check on stdout
- `-G` with `--data-urlencode` for every parameter
- non-zero exit, timeout, oversized body, or non-UTF-8 body becomes a typed
  `WeatherUnavailable`

The public pure parser is:

```rust
pub fn parse_open_meteo(
    json: &str,
    request: &WeatherRequest,
) -> Result<WeatherData, WeatherUnavailable>
```

It produces contract types `WeatherObservation`, `WeatherForecast`,
`SunTimes`, and `WeatherData`, and runs the contract `validate()` methods before
returning. WMO codes are mapped to human conditions by
`pub fn wmo_condition(code: i64) -> &'static str` (0 -> clear sky, 61 -> slight
rain, 95 -> thunderstorm, unknown -> "unknown conditions", etc.).

Strictness:

- response must be a JSON object with a matching `timezone`;
- `utc_offset_seconds` must be present and within +/-18h;
- every requested current field and every daily array must be present;
- all daily arrays must have the same non-zero length (<= 16);
- JSON types, finite numbers, humidity/probability 0-100, and contract bounds
  are enforced;
- a malformed, partial, or validation-failing payload returns
  `WeatherUnavailable` with a reason bounded to 200 characters.

`observed_at` is emitted as RFC 3339 in the requested zone, e.g.
`2026-11-26T13:00:00-06:00`, built from Open-Meteo's local `current.time` and
`utc_offset_seconds`. It is never replaced with the fetch time.

Two requested fields are validated but cannot be surfaced because the M2
contract has no slots for them: `precipitation` (mm) and `is_day`.

---

## 3. Cache, freshness, and cached-vs-live

- In-memory only, per process: `Mutex<HashMap<key, CacheEntry>>`.
- Key: exact `latitude.to_bits()` + `longitude.to_bits()` + IANA timezone.
- Capacity: 16 entries; the oldest entry is evicted when full.
- TTL: 900 seconds (`OPEN_METEO_CACHE_TTL_SECONDS`). `with_cache_ttl()` is
  clamped to 900 seconds so a hit can never be older than 15 minutes.
- A hit returns the original `WeatherData` clone, including the original
  `observation.observed_at`; it does not rewrite the observation time.
- An expired entry is removed and a live request is attempted.
- On live failure the provider returns `WeatherUnavailable`; it does **not**
  fall back to expired cache data, so stale data is never labeled current.

The M2 `WorldService` still owns snapshot freshness: it records
`provenance.retrieved_at = snapshot.generated_at` and computes freshness from
`generated_at` plus `stale_after_seconds` in `world.rs`. M6 does not modify
M2. Consequently the cached/live distinction is visible through the preserved
`observation.observed_at` versus the snapshot generation time, while the
M2 freshness field remains governed by snapshot age. Because the cache TTL is
capped at the M2 default (900 s), an expired observation cannot be returned as
a cache hit.

Cache failures do not poison the cache: the mutex is recovered from poisoning
with `into_inner()` rather than panicking.

---

## 4. Failure handling

| Condition | Result |
|---|---|
| `latitude` or `longitude` is `None` | `WeatherUnavailable("location not configured")` |
| Location is non-finite or out of range | `WeatherUnavailable("location is invalid")` |
| Timezone empty, control-containing, or over 64 chars | `WeatherUnavailable("timezone is invalid")` |
| `curl.exe` missing / cannot spawn | Typed reason `weather transport failed: ...` |
| Network failure, timeout, HTTP error (`--fail`) | Typed reason with bounded exit code / stderr detail |
| Body over 64 KiB or non-UTF-8 | Typed size / encoding reason |
| Malformed JSON, missing/partial field, wrong type | Typed reason naming the field |
| Contract bounds violated | Typed validation reason |
| API response `timezone` differs from request | Typed mismatch reason |

All reasons are control-character-free and truncated to 200 characters before
M2 further bounds provenance notes to 240 characters. `WorldService` converts
any `Err(WeatherUnavailable)` into an `Unavailable` snapshot with empty weather,
so the Platform keeps working and no weather is invented.

---

## 5. Verification

### Tests

`cargo test -p aegis-platform`:

- 25 tests passed, 0 failed
  - `src/lib.rs` weather unit tests: 6
  - `tests/platform_foundation.rs`: 6
  - `tests/weather.rs`: 4
  - `tests/world.rs`: 9
  - doc-tests: 0

M6 tests cover:

- fixture parsing into all three contract types;
- malformed JSON, non-object JSON, missing `current`, missing temperature,
  timezone mismatch;
- WMO code mapping (known and unknown);
- missing and invalid location failing before any network call;
- cache freshness, expiry, original `observed_at` preservation, size bound,
  oldest-entry eviction, and TTL clamping;
- integration-level parser/provider tests plus the env-gated live test.

### Clippy

`cargo clippy -p aegis-platform --all-targets -- -D warnings`:

- Finished with no warnings or errors.

### Live evidence

Command:

```powershell
$env:AEGIS_WEATHER_LIVE='1'
cargo test -p aegis-platform --test weather live_open_meteo_observation_when_enabled -- --nocapture
Remove-Item Env:\AEGIS_WEATHER_LIVE
```

Result (fixed location 41.88, -87.63, America/Chicago; run 2026-11-26):

```text
live weather source=open-meteo condition=slight rain temperature_c=Some(6.0)
feels_like_c=Some(0.4) wind_kph=Some(29.3) humidity_pct=Some(88)
observed_at=Some("2026-11-26T13:00:00-06:00") forecast_days=3
sunrise=Some("2026-11-26T06:53") sunset=Some("2026-11-26T16:22")
test live_open_meteo_observation_when_enabled ... ok
```

This is a live Open-Meteo response, not synthetic data. When the environment
variable is unset the test is skipped. When the network fails the test prints
the exact `WeatherUnavailable` reason and returns without panicking, so offline
CI remains green while the reason is still visible.

---

## 6. Limits and next steps

Limits:

- Requires `curl.exe` on `PATH` (verified locally: curl 8.23.0). This is a
  deliberate choice to avoid new Rust dependencies.
- Depends on Open-Meteo availability, fair-use rate limits, and network access.
  The 900 s cache reduces call volume; failed calls are not retried in a loop.
- Cache is process-local and lost on restart; no disk persistence.
- Only current conditions plus 3 daily forecast days; no hourly or air quality.
- Contracts do not carry precipitation amount or `is_day`.
- The response timezone must exactly match the request timezone; alias
  differences are rejected rather than guessed.
- No API key is configured, which is correct for Open-Meteo's free endpoint.

Next steps:

- Compose `OpenMeteoProvider` into the Platform/CLI startup path that currently
  defaults to `UnavailableWeatherProvider`.
- Optionally surface a cache-hit flag in provenance so operators can distinguish
  cached from live in logs without changing contracts.
- Align any future configurable weather TTL with `world.stale_after_seconds`.
- If dependencies become acceptable later, replace the `curl.exe` shell-out
  with an in-process HTTPS client behind the same `WeatherProvider` trait.
