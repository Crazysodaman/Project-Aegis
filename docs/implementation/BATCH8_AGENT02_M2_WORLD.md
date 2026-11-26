# Batch 8 - Agent M2: World Service (Milestone 2)

**Date:** 2026-11-26
**Baseline:** HEAD `2bd2513`, all existing uncommitted Batch 3-7 work preserved.
**Scope:** Platform-owned shared world clock, bounded world snapshot contracts,
honest weather-provider boundary, and platform.db persistence.

---

## 1. Status

Implemented. No commits, resets, rebases, deploys, or workspace-wide gates were
run. No files outside the assigned allowlist were modified. `chrono-tz` (already
added to `aegis-platform` by the Lead) is used for IANA/DST correctness.

### Files

| File | Change |
|---|---|
| `crates/aegis-contracts/src/world.rs` | New versioned, authority-free world contract types and validation. |
| `crates/aegis-contracts/src/lib.rs` | Added `pub mod world;` only. |
| `crates/aegis-platform/src/world.rs` | New Platform `WorldService`, `WorldClock`, `WeatherProvider`, and `WorldStore`. |
| `crates/aegis-platform/src/lib.rs` | Added `mod world;` and public re-exports only. |
| `crates/aegis-platform/tests/world.rs` | New world-service integration tests. |
| `docs/implementation/BATCH8_AGENT02_M2_WORLD.md` | This document. |

---

## 2. Architecture

The Platform is the single authority for shared world time. Cores never open
`platform.db`; callers ask `WorldService` for a bounded `WorldSnapshot` and pass
that same snapshot to Cores.

```
platform_config (platform.db)
        |
        v
WorldConfig -> WorldClock (chrono + chrono-tz) -> WorldClockReading
                                   |
WeatherProvider -------------------+--> WorldSnapshot
                                   |
                                   v
WorldStore (platform.db): world_context + append-only world_events
                                   |
                                   v
snapshot_for_core()/shared_snapshot() -> every Core sees the same stored data
```

### Contracts (`aegis-contracts/src/world.rs`)

- `WORLD_SNAPSHOT_SCHEMA = "aegis.world.snapshot.v1"`.
- Enums: `TimeOfDay`, `Season`, `FreshnessState`.
- Value types: `Provenance`, `SunTimes`, `WeatherObservation`,
  `WeatherForecast`, `CalendarEvent`, `WorldSnapshot`.
- `WorldSnapshot::validate()` enforces:
  - schema, non-empty/bounded timestamps, timezone, local date/time;
  - UTC offset within +/-18 hours (IANA reality is +/-14);
  - forecast <= 16 entries and calendar <= 64 events;
  - bounded/control-free strings, finite temperature range, humidity <= 100,
    wind speed range, forecast precipitation <= 100;
  - stale-after within 1 second .. 7 days.
- No authority, credentials, secrets, or personal Core state are represented.

`TimeOfDay::from_local_hour` is deterministic: Night 22:00-04:59, Dawn
05:00-06:59, Morning 07:00-11:59, Midday 12:00-12:59, Afternoon
13:00-16:59, Evening 17:00-19:59, Dusk 20:00-21:59.

`Season::from_month` is meteorological and hemisphere-aware. The Platform
derives the hemisphere from configured latitude: latitude >= 0 (or missing
latitude) is Northern; negative latitude is Southern.

---

## 3. Configuration keys

All keys live in the existing additive `platform_config` table in
`platform.db`.

| Key | Default | Notes |
|---|---|---|
| `world.timezone` | `"UTC"` | IANA name validated with `chrono_tz::Tz::from_str`. |
| `world.latitude` | unset | Decimal degrees, `-90.0..=90.0`; must be paired with longitude. |
| `world.longitude` | unset | Decimal degrees, `-180.0..=180.0`; must be paired with latitude. |
| `world.stale_after_seconds` | `900` | Contract bound 1 second .. 7 days. |

Owner-gated setters (Safe owner proof required):

- `WorldService::set_timezone(owner_token, timezone)`
- `WorldService::set_location(owner_token, latitude, longitude)`

A wrong owner token returns `WorldError::Unauthorized`; invalid IANA/location
values are rejected before persistence.

---

## 4. Clock and DST evidence

`WorldClock::clock_at(DateTime<Utc>)` is pure for tests and returns local
RFC3339 date/time, UTC offset, `TimeOfDay`, and `Season`. `WorldClock::now()`
uses `chrono::Utc::now()`. IANA conversion uses:

```rust
let tz = chrono_tz::Tz::from_str(&timezone)?;
let local = instant.with_timezone(&tz);
let offset = local.offset().fix().local_minus_utc();
```

`WorldService::clock()` builds the clock from persisted config.

### Fixed-instant evidence (`America/Chicago`)

| UTC instant | Expected local | Offset | Verified |
|---|---|---|---|
| `2026-03-08T07:59:59Z` | `01:59:59` | `-06:00` (CST) | `chicago_dst_spring_forward_and_fall_back_are_correct` |
| `2026-03-08T08:00:00Z` | `03:00:00` | `-05:00` (CDT) | same test (02:00 skips to 03:00) |
| `2026-11-01T06:59:59Z` | `01:59:59` | `-05:00` (CDT) | same test |
| `2026-11-01T07:00:00Z` | `01:00:00` | `-06:00` (CST) | same test (01:00 repeats) |

Additional clock evidence:

- `2026-07-15T12:00:00Z` in UTC => Midday, Northern Summer.
- `2026-07-15T02:00:00Z` in `Australia/Sydney` => local `12:00:00`, Midday,
  Southern Winter (negative latitude).

---

## 5. Weather provider interface

```rust
pub struct WeatherRequest {
    pub timezone: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub now: DateTime<Utc>,
}

pub struct WeatherData {
    pub observation: Option<WeatherObservation>,
    pub forecast: Vec<WeatherForecast>,
    pub sun: Option<SunTimes>,
}

pub struct WeatherUnavailable {
    pub reason: String,
}

pub trait WeatherProvider {
    fn name(&self) -> &'static str;
    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable>;
}
```

`UnavailableWeatherProvider` is the honest default. It performs no network I/O
and returns `Err(WeatherUnavailable)` with a reason.

Provider failures never break snapshotting:

- `weather = None`, `forecast = []`, `sun = None`;
- `provenance.source = "unavailable"`, provider name recorded, reason bounded
  into `provenance.note`;
- `freshness = Unavailable`;
- clock/date/time/season are still computed and the snapshot is stored.

Malformed provider data (for example humidity > 100 or oversized strings) is
rejected by contract validation and falls back to the same honest unavailable
snapshot instead of failing the Platform.

---

## 6. Persistence table

Additive tables in `platform.db` (backup already covers `platform.db`):

| Table | Shape | Semantics |
|---|---|---|
| `world_context` | `singleton INTEGER PRIMARY KEY CHECK(singleton = 1)`, `snapshot_json TEXT NOT NULL`, `updated_at TEXT NOT NULL` | Current snapshot; replaced atomically. |
| `world_events` | `sequence INTEGER PRIMARY KEY AUTOINCREMENT`, `event_type TEXT NOT NULL`, `detail_json TEXT NOT NULL`, `created_at TEXT NOT NULL` | Append-only history (`world.snapshot` events). |

`WorldStore::store` uses a SQLite transaction: it upserts `world_context` and
appends one `world_events` row, then commits. `latest`/`latest_at` read the
current row and recompute freshness from `generated_at` and
`stale_after_seconds` (Fresh when age <= threshold, Stale afterward, Unavailable
when no weather/sun/forecast data exists).

`WorldService::snapshot_for_core` / `shared_snapshot` read the single stored
snapshot, so multiple Cores receive identical data. If no snapshot exists yet,
`snapshot_for_core` builds and stores one.

---

## 7. Tests and results

Commands run (scoped only):

```
cargo test -p aegis-contracts
cargo test -p aegis-platform
cargo clippy -p aegis-platform --all-targets -- -D warnings
```

| Suite | Result |
|---|---|
| `cargo test -p aegis-contracts` | 20 passed, 0 failed (5 new world contract tests). |
| `cargo test -p aegis-platform` | 15 passed, 0 failed (6 existing foundation + 9 new world tests). |
| `cargo clippy -p aegis-platform --all-targets -- -D warnings` | Clean. |

New Platform tests:

1. `chicago_dst_spring_forward_and_fall_back_are_correct`
2. `time_of_day_and_season_follow_local_clock_and_hemisphere`
3. `config_defaults_and_owner_gated_timezone_and_location`
4. `snapshot_is_valid_stored_and_recomputes_freshness`
5. `snapshot_store_survives_reopen`
6. `unavailable_provider_is_honest_and_does_not_break_snapshotting`
7. `malformed_provider_data_falls_back_to_unavailable`
8. `multiple_cores_receive_the_same_shared_snapshot`
9. `world_snapshot_contract_rejects_unbounded_values`

New contract tests: valid snapshot, time-of-day mapping, hemisphere seasons,
forecast/calendar bounds, timezone/offset/stale/string bounds.

---

## 8. What M6 will add

M6 owns weather network calls and any provider credentials/API keys. It should:

- implement `WeatherProvider` for the approved network source;
- populate `observation`, `forecast` (<= 16 entries), and `sun`
  (`SunTimes`) from the provider;
- keep failure/malformed responses mapped to `WeatherUnavailable` so this
  service retains the honest unavailable fallback.

No network call, API key, HTTP client, or new dependency was added here.

---

## 9. Limitations

- Sunrise/sunset are intentionally left to the weather provider; the clock does
  not calculate astronomy locally.
- No calendar source is implemented yet. `CalendarEvent` and the 64-event bound
  are ready for a future calendar provider/caller; the service emits an empty
  calendar today.
- `freshness` describes shared weather/sun/forecast availability and age. Clock
  fields are always computed even when weather is unavailable.
- Hemisphere is derived from latitude sign (Northern default when latitude is
  unset); there is no separate hemisphere override key.
- Distribution is service-level (`snapshot_for_core`/`shared_snapshot`); no new
  IPC method or Core SDK surface was added in this milestone.
- No personal state, emotion, continuity, wardrobe decisions, or personal
  persistence were added; M3 owns those.
