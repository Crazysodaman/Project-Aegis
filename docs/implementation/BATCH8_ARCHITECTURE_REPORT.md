# Batch 8 — Architecture Report: World & Continuity

**Date:** 2026-11-26

## 1. Ownership model

- **Platform owns shared world**: authoritative UTC clock, IANA timezone
  configuration, local time/time-of-day/season, optional weather/forecast/sun,
  calendar context, provenance, freshness, and distribution snapshots.
- **Each Core owns personal state**: emotion (current + history), continuity,
  activities, routines, outfit decisions, preferences. Stored per Core; never
  shared implicitly.
- Shared world information is not private memory; a Core may record its own
  interpretation as a private experience later.

## 2. Contracts (`aegis-contracts`)

- `world.rs`: `WorldSnapshot`, `TimeOfDay`, `Season`, `FreshnessState`,
  `Provenance`, `SunTimes`, `WeatherObservation`, `WeatherForecast`,
  `CalendarEvent`; bounded validation (forecast ≤16, calendar ≤64, string caps,
  offset range, stale window).
- `personal.rs`: `EmotionDimensions` (valence/arousal/joy/calm/curiosity/tension),
  `EmotionState`, `EmotionDelta`, `EmotionInfluence`, `Activity`,
  `ContinuitySummary`, `Routine`, `OutfitDecisionRecord`; bounded validation.
- Contracts are authority-free; no secrets; no policy.

## 3. World Service (`aegis-platform::world`)

- `WorldConfig` from `platform_config`: `world.timezone` (IANA, default UTC),
  `world.latitude`/`world.longitude` (paired, owner-gated),
  `world.stale_after_seconds` (default 900).
- `WorldClock` uses `chrono-tz::Tz` + `chrono::Utc`; `clock_at` is pure for
  tests; DST handled by the tz database. Hemisphere from latitude sign.
- `WorldStore` in `platform.db`: singleton `world_context` + append-only
  `world_events`; atomic transactional writes; `latest`/`latest_at` recompute
  freshness.
- `WeatherProvider` trait: `observe(&WeatherRequest) -> Result<WeatherData,
  WeatherUnavailable>`; `UnavailableWeatherProvider` is the honest default.
  Provider failure yields no weather, not a false observation.
- `WorldService::snapshot(provider)` builds/stores a snapshot; shared for all
  Cores via `snapshot_for_core`/`shared_snapshot`.

## 4. Personal State (`aegis-core-sdk::personal`)

- New `<core>/personal.db`, app id `AEGL`, component `core_personal`.
- Ownership: exactly one `personal_ownership` row with `core_uuid`,
  `database_instance_uuid`, `schema`, `created_at`; immutable triggers. Opening
  reads `core.db` read-only and rejects copies/mismatches.
- Tables: `personal_ownership`, `personal_schema`, `emotion_state`,
  `emotion_history` (append-only), `continuity`, `activities`, `routines`,
  `outfit_decisions` (append-only), `personal_events` (append-only),
  `open_threads`.
- Emotion engine: decay half-life 2700 s toward neutral; weighted influences;
  personality factor from optional traits; intensity = 0.4·|valence| +
  0.6·mean(other dimensions); deterministic, no startup randomness.
- Continuity: `last_active_at`, `last_session_id`, `elapsed_offline_seconds`,
  bounded recent experiences/open threads/activities.
- `CoreSdkContext::personal()` exposes the store without changing existing APIs.

## 5. Conversation integration (`aegis-conversation`)

- `TurnOptions` gains `world`, `continuity`, `emotion`, `activities` (+ builders).
- `compose_system_prompt` appends bounded sections: world (time/season/weather
  with source+observed_at+freshness; stale/unavailable stated), continuity
  (elapsed, recent experiences, open threads, activities), internal emotional
  dimensions (no single mood label). Hard cap 8,000 chars, UTF-8 safe.
- Turn reports flags `world_used/continuity_used/emotion_used/activities_used`;
  after a successful turn `update_last_active(session)` runs; failures are
  non-fatal (`personal_update_warning`).

## 6. Embodiment (`aegis-embodiment::presentation`)

- `PresentationContext` + deterministic `select_outfit` (wardrobe assets only,
  ownership/live/kind enforced; weather/season/time/activity/emotion/personality
  scoring; hard safety/identity constraints; explicit unavailable when empty).
- `voice_projection` (rate 60–150%, volume 40–100%, style label) and
  `expression_for_emotion` (only expressions already stored in appearance).
- Optional BMP presentation frame via existing rasterizer. No new renderer.

## 7. Weather (`aegis-platform::weather`)

- `OpenMeteoProvider` shells out to `curl.exe` (argv, no shell) for HTTPS
  Open-Meteo; no API key; no Rust TLS dependency.
- Parses current + 3-day forecast + sunrise/sunset; WMO code mapping; strict
  validation; 900 s bounded in-memory cache; cached entries preserve
  `observed_at` so freshness can mark stale. Missing location / network / HTTP /
  JSON failures return typed `WeatherUnavailable`.

## 8. Backup & recovery (`aegis-backup`, format v2)

- Per-Core inventory: `core.db`, `core.integrity.json`, `memory.db`,
  `conversation.db`, `assets.db`, `social.db`, `embeddings.db`, `personal.db`
  (optional), init marker, and referenced asset files under `assets/`.
- Manifest v2 records kind/sha256/bytes and per-Core database lists; v1 manifests
  remain readable.
- Ownership verified per DB (core UUID; core.db also instance UUID); digest
  verified on backup and restore; temp+atomic writes; partial restore refused.

## 9. Dependency decision

`chrono-tz = 0.10.0` added (workspace + platform) because IANA identifiers and
DST cannot be implemented correctly with a fixed offset. This is the only new
third-party crate. Weather uses the system `curl.exe` to avoid a TLS/HTTP crate.

## 10. Invariants and limitations

Invariants: no invented weather; no random emotion at startup; per-Core personal
isolation; immutable ownership rows; bounded prompt/context; no identity mutation.
Limitations: no automatic event→emotion influence yet; no IPC background
distribution; no SAPI use of voice projection; no image/GPU renderer; backup not
live-write-consistent (SQLite backup API not used); multi-host distribution
blocked on Artemis authorization.
