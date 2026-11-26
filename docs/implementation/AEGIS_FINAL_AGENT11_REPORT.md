# AEGIS FINAL — Agent 11 (GUI & UI Integration)

**Date:** 2026-11-26
**Baseline:** HEAD `2bd2513` (shared uncommitted Batch 3–8 work preserved)
**Surface:** loopback HTTP dashboard only — **not** a native window.

## Files

- `crates/aegis-ui/src/gui.rs`
  - Added `World` plus `Personal` / `Emotion` / `Continuity` / `Outfit`
    sections to both the dashboard index (`/`) and every Core workspace
    (`/core/<uuid>` and `/api/core/<uuid>`).
  - Reuses the existing shared model loader (`model::DashboardModel`) for
    world/personal freshness and values. No parallel snapshot parser.
  - `Outfit` is a GUI-only extension read from the same fresh personal
    snapshot (the shared model does not model outfit decisions).
  - `/api/status` now also reports `snapshots.world` and `snapshots.personal`
    states/reasons.
- `crates/aegis-ui/tests/gui_dashboard.rs`
  - Appended 6 integration tests (tempdirs + loopback server) and extended the
    existing exact section-title expectation to include the new sections.
- `docs/implementation/AEGIS_FINAL_AGENT11_REPORT.md` — this report.

`html.rs` and `actions.rs` were left unchanged: the existing renderer already
escapes every section title/note/row/item, and the existing controls are all
wired to real actions. **No new controls or dead controls were added.**

## Behavior (honest states)

- Snapshot freshness is the shared `model::STATUS_SNAPSHOT_MAX_AGE_SECS = 60`.
  Age is taken from `updated_at` (RFC 3339) when present, else file mtime.
- `missing` file → **UNAVAILABLE** (explicit reason).
- `malformed` JSON / wrong field types → **UNAVAILABLE** (explicit reason).
- older than 60 s → **STALE**; no values from the stale file are rendered.
- fresh file with missing per-section data → that section is **UNAVAILABLE**
  (Emotion/Continuity/Outfit are never inferred available).
- fresh file with no usable world/personal fields → **PARTIAL**.
- All snapshot text is passed through the existing `escape_html` renderer.

## Exact snapshot schemas

### `<root>/run/status/world.json`

JSON object; every field is optional. Unknown fields are ignored.

```json
{
  "updated_at": "2026-11-26T12:00:00Z",
  "local_time": "08:30",
  "timezone": "UTC",
  "time_of_day": "morning",
  "season": "autumn",
  "weather": {
    "condition": "clear sky",
    "temperature_celsius": 12.5
  }
}
```

- `updated_at`: RFC 3339; freshness source (falls back to file mtime).
- `local_time`, `timezone`, `time_of_day`, `season`: strings shown verbatim.
- `weather.condition` (string) and `weather.temperature_celsius` (number) are
  shown as `Condition` / `Temperature (C)`.
- `condition` and `temperature_celsius` are also accepted at the top level.
- `updated_at` is ignored by the renderer; it only controls freshness.

### `<root>/run/status/personal.json`

JSON object; every field is optional. Unknown fields are ignored by the shared
model.

```json
{
  "updated_at": "2026-11-26T12:00:00Z",
  "core_uuid": "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d",
  "emotion": {
    "valence": 0.4,
    "arousal": 0.0,
    "joy": 0.0,
    "calm": 0.8,
    "curiosity": 0.0,
    "tension": 0.0
  },
  "continuity": {
    "summary": "kept a journal",
    "recent_experiences": ["..."],
    "elapsed_offline_seconds": 10
  },
  "activities": [{ "title": "Reading" }],
  "routines": [{ "name": "Morning check-in" }],
  "outfit": {
    "id": "outfit-1",
    "decided_at": "2026-11-26T12:00:00Z",
    "asset_ids": ["asset-1", "asset-2"],
    "rationale": "warm and simple"
  }
}
```

- `updated_at`: RFC 3339; freshness source (falls back to file mtime).
- **Personal** section: `core_uuid` plus `activities` / `routines` array counts.
- **Emotion** section: `emotion` object dimensions formatted as
  `valence=0.40 arousal=0.00 ...` (only present dimensions), or the
  `emotion_summary` string.
- **Continuity** section: `continuity.summary`, else a summary derived from
  `recent_experiences` / `elapsed_offline_seconds`, or `continuity_summary`.
- **Outfit** section (GUI extension): `outfit` may be a plain string summary,
  or an object validated against
  `aegis_contracts::personal::OutfitDecisionRecord` (`id`, `decided_at`,
  `asset_ids` ≤32, `rationale` ≤1024). If absent → UNAVAILABLE.
- A missing/invalid `emotion` or `continuity` only disables that section; it does
  not mark the others available by inference.

## Verification

```
cargo test -p aegis-ui
cargo clippy -p aegis-ui --all-targets -- -D warnings
```

- Lib unit tests: **22 passed**.
- `tests/gui_dashboard.rs`: **37 passed** (6 new).
- `tests/tui_dashboard.rs`: **13 passed** (shared model loader).
- Total: **72 passed, 0 failed**.
- Clippy: **clean** (`-D warnings`, all targets).

New GUI tests:

1. `dashboard_renders_fresh_world_and_personal_snapshots`
2. `stale_world_and_personal_snapshots_render_stale_not_values`
3. `missing_world_and_personal_snapshots_are_unavailable`
4. `malformed_world_and_personal_snapshots_are_unavailable_without_panic`
5. `fresh_personal_snapshot_missing_emotion_is_not_inferred_available`
6. `world_and_personal_snapshot_text_is_html_escaped`

## Wired vs unavailable

- **Wired (real data):** Platform status, diagnostics, Core roster, per-Core
  runtime/heartbeat/integrity, Core lifecycle history, module registry,
  platform-wide declared permissions, Fleet/GPU/Inference/Service snapshots,
  and World / Personal / Emotion / Continuity / Outfit snapshots.
- **Unavailable / partial (explicit):** per-Core Memory, Personality, Voice,
  Appearance, per-Core permission grants, GPU when the snapshot reports it
  unavailable, and any missing/stale/malformed snapshot section.
- `/api/status` exposes the same snapshot states with machine-readable reasons.

## Blockers / limits

- **Native window remains blocked:** no approved windowing/GUI dependency is
  present in the workspace, and adding one is out of scope. The dashboard stays a
  loopback-only HTTP page.
- **No in-tree snapshot emitter:** `world.json` / `personal.json` must be written
  by an operator or CLI; absence is reported as UNAVAILABLE, never synthesized.
- **Outfit is GUI-only:** the shared TUI/model view does not render outfit
  decisions yet; a shared schema decision is still pending.
- World distribution across hosts (Fleet/Artemis) is not wired.
