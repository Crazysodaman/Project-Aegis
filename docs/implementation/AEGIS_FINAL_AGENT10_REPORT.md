# Aegis Final — Agent 10 (Appearance & Wardrobe)

Scope: `crates/aegis-embodiment` only, HEAD `2bd2513`. No other crates, Core
appearances, approved identity data, CLI, workspace, or
`D:\SofiaDev\SofiaAdaLyra` were touched. No new dependencies, no unsafe, no
commit/reset/clean/deploy. `lib.rs` was not edited.

## Files

- `crates/aegis-embodiment/src/presentation.rs` — hardened `select_outfit()`:
  hard identity tags, stricter deterministic scoring/tie-breaks, bounded
  explainable rationale, and additive
  `OutfitDecision::to_personal_record()` conversion.
- `crates/aegis-embodiment/tests/presentation.rs` — 4 new integration tests
  (13 total in this file).
- `docs/implementation/AEGIS_FINAL_AGENT10_REPORT.md` — this report.

`appearance.rs`, `asset.rs`, `store.rs` were reviewed but needed no changes:
the selector still reads only Core-owned, live `Wardrobe` assets and their
existing name/tags, and never mutates appearance, assets, or wardrobe.

## Behavior

Selection remains pure and deterministic: same store + same context always
produce the same decision. Hard constraints run **before** any preference score:

1. `core-ownership` — caller Core UUID must match the opened store.
2. `live-only` — soft-deleted wardrobe assets are rejected.
3. `wardrobe-kind` — non-wardrobe assets are never considered.
4. `cold-safety` / `heat-safety` / `precipitation-safety` — from optional
   feels-like/temperature, condition words, and humidity.
5. `required-tags` (identity) — every caller-supplied required tag must be
   present (case-insensitive exact tag), e.g. protective/identity `armor`; an
   `armor-identity` constraint marker is recorded when `armor` is required.

Only after these gates do weather comfort, season, time-of-day, activity,
emotion, personality, and preference signals add to the score. Ties break by
lowercased name then asset id. Therefore weather safety and identity/armor
requirements can never be overridden by personality or preferences.

`OutfitDecision` still reports `schema`, `core_uuid`, `available`, `asset_ids`,
bounded `rationale`, applied `constraints`, structured `rejected` reasons, and
`considered`. Empty or fully rejected inventory is honest: `available = false`,
empty `asset_ids`, and no invented assets.

### Additive personal-record conversion

```rust
decision.to_personal_record(id, decided_at) -> OutfitDecisionRecord
```

Copies selected asset ids and rationale into
`aegis_contracts::personal::OutfitDecisionRecord`; all fields are truncated to
the shared contract bounds (id, timestamp, asset ids, rationale). This is an
in-memory conversion only. Embodiment has **no** `aegis-core-sdk` dependency.
**Persistence and any UI wiring are caller-owned**: the caller supplies the
record id/timestamp and persists it through its own personal store.

## Verification

- `cargo test -p aegis-embodiment`: **128 passed, 0 failed, 1 ignored**
  (ignored test is the pre-existing LAN/artifact-node test).
  - `tests/presentation.rs`: 13 passed, including 4 new tests:
    - `required_identity_tags_and_weather_safety_beat_preference`
    - `empty_inventory_is_honest_and_never_invents_assets`
    - `outfit_scoring_is_deterministic_across_repeated_calls`
    - `outfit_decisions_convert_to_bounded_personal_records`
  - Existing coverage retained: two Cores differ, no cross-Core leakage,
    weather/season/time changes selection, deleted/non-wardrobe handling, limits.
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings`: **clean**.

## Hard constraints (summary)

- Only Core-owned, live `Wardrobe` assets can be selected; foreign ids cannot
  appear in a decision.
- Weather safety gates require a matching warm/cool/wet tag under cold/heat/
  precipitation and reject unsafe assets before scoring.
- Required identity tags (including `armor`) are unconditional gates, not score
  bonuses.
- No approved appearance, asset content, wardrobe list, or Core identity is
  changed.

## Limitations

- Tags/name are still the only clothing metadata; scoring is a documented
  keyword/tag heuristic, not a fabric/thermal/physics model.
- Environmental safety uses optional weather fields; absent weather applies no
  environmental gate. Season alone affects soft scoring.
- Required tags are caller-supplied strings; the embodiment layer does not own a
  policy or profile store and does not infer identity requirements.
- Preferences/personality are caller-supplied tag slices; no learning/history.
- Rationale text is bounded and explainable but not a full score breakdown.
- Conversion does not persist or display anything; that remains caller-owned.
