# Batch 8 — Agent M3: Personal State & Emotional Continuity

Status: **IMPLEMENTED / TESTED**
HEAD: `2bd2513` (all pre-existing uncommitted work preserved; nothing was reset, cleaned, or committed)

Files changed (only these):

| File | Change |
| --- | --- |
| `crates/aegis-contracts/src/personal.rs` | **new** versioned, authority-free personal contracts |
| `crates/aegis-contracts/src/lib.rs` | module declaration only |
| `crates/aegis-core-sdk/src/personal.rs` | **new** `PersonalStore`, emotion engine, continuity/activity/routine/outfit APIs |
| `crates/aegis-core-sdk/src/lib.rs` | module/re-export + `CoreSdkContext` integration only |
| `crates/aegis-core-sdk/tests/personal.rs` | **new** 12 integration tests |
| `docs/implementation/BATCH8_AGENT03_M3_PERSONAL.md` | this document |

No workspace manifest, `Cargo.lock`, `core.db`, `memory.db`, other crate, `world.rs`, platform, or completion-ledger changes were made. No new dependencies were added.

## 1. Contract types (`aegis-contracts/src/personal.rs`)

All types are serializable, descriptive data only. None of them carries a role, permission, credential, or authorization input.

| Type | Purpose | Bounds |
| --- | --- | --- |
| `EmotionDimensions` | `valence` (−1..=1), `arousal`/`joy`/`calm`/`curiosity`/`tension` (0..=1) | finite floats only |
| `EmotionState` | `schema`, `core_uuid`, `dimensions`, `intensity` (0..=1), `updated_at`, `revision` | revision ≥ 1, RFC3339-ish timestamp ≤ 64 chars |
| `EmotionDelta` | signed deltas for the six dimensions | each −1..=1 |
| `EmotionInfluence` | `source`, `weight` (0..=1), `delta` | source ≤ 128 chars |
| `Activity` | `id`, `title`, status, `started_at`, `updated_at`, `notes` | title ≤ 160, notes ≤ 2048 |
| `Routine` | `id`, `name`, optional `time_of_day`, `notes` | name ≤ 160, time ≤ 64, notes ≤ 1024 |
| `ContinuitySummary` | last active/session, elapsed offline, ≤ 8 experiences, ≤ 8 threads, ≤ 8 activities | elapsed ≤ 10 years |
| `OutfitDecisionRecord` | `id`, `decided_at`, asset ids, rationale | ≤ 32 assets, rationale ≤ 1024 |

Constants include `PERSONAL_SCHEMA`, `EMOTION_STATE_SCHEMA`, `CONTINUITY_SUMMARY_SCHEMA`, `ACTIVITY_SCHEMA`, `ROUTINE_SCHEMA`, `OUTFIT_DECISION_SCHEMA`, and all `MAX_*` bounds. `ActivityStatus::{Planned, Active, Paused, Completed, Cancelled}` serializes to stable snake_case strings.

Validation rejects non-finite/out-of-range floats, empty required strings, control characters, oversized strings, oversized collections, invalid Core UUIDs, and invalid revisions.

## 2. Personal database schema (`personal.db`)

`PersonalStore::open` creates/opens `<cores>/<core_uuid>/personal.db` with SQLite application marker `0x4145_474C` (`AEGL`) and component `core_personal`. The file must be a regular, non-symlinked file directly inside the Core directory.

Tables:

* `personal_ownership` — exactly **one** row: `singleton = 1`, `core_uuid`, `database_instance_uuid`, `schema`, `created_at`.
* `personal_schema` — one schema-version marker row: `schema`, `revision = 1`, `created_at`.
* `emotion_state` — one row per Core: six dimensions, intensity, timestamp, revision.
* `emotion_history` — append-only snapshots with `sequence`, `revision`, `source`, `created_at`.
* `continuity` — one row per Core: `last_active_at`, `last_session_id`, `updated_at`.
* `activities` — upserted activities with stable status strings.
* `routines` — upserted routines.
* `outfit_decisions` — append-only wardrobe decisions (`asset_ids_json`).
* `personal_events` — append-only personal event log (`kind`, `detail`, `session_id`).
* `open_threads` — thread summaries with `open`/`resolved` status.

Immutable append-only enforcement is implemented with SQLite triggers:

* `personal_ownership` — no UPDATE, no DELETE (ownership is immutable).
* `emotion_history`, `outfit_decisions`, `personal_events` — no UPDATE, no DELETE.

All table creation uses `CREATE TABLE IF NOT EXISTS` and `CREATE TRIGGER IF NOT EXISTS`, so opening an existing database is additive. `core.db` and `memory.db` are never written by this store.

## 3. Ownership binding

On open:

1. Read `core.db` read-only through `CoreDatabase::read_snapshot` (identity + `database_instance_uuid`).
2. Require the snapshot Core UUID to equal the requested Core.
3. If `personal.db` already existed, require `personal_ownership` to exist and contain exactly one row.
4. Require the stored `core_uuid` to match the Core UUID **and** the stored `database_instance_uuid` to match the Core database instance UUID.
5. Reject an existing `personal.db` with a missing ownership table, foreign Core, wrong instance UUID, or unsupported schema.

Copying `personal.db` between Cores is rejected with `SdkError::OwnershipMismatch`. A hand-written ownership row with a valid but wrong database instance UUID is also rejected.

## 4. Emotion engine (pure, deterministic)

No randomness is used anywhere in emotion generation. A fresh Core starts fully neutral (all dimensions 0, intensity 0). The same stored state, influences, personality, and timestamps always produce the same result.

**Decay.** Every dimension decays exponentially toward zero with a documented half-life of **45 minutes** (`EMOTION_DECAY_HALF_LIFE_SECONDS = 2700`):

```text
factor = 0.5 ^ (elapsed_seconds / 2700)
dimension' = dimension * factor
```

Zero elapsed time applies no decay. Long offline periods asymptotically approach the neutral state.

**Influences.** Each `EmotionInfluence` is validated, then applied as a weighted signed delta:

```text
effective_weight = clamp(influence.weight * personality_factor, 0.0, 1.0)
dimension' = dimension' + influence.delta.dimension * effective_weight
```

Multiple influences are applied in caller order (summation is commutative; order only affects intermediate rounding).

**Personality modulation (read-only, bounded).** If `memory.db` exists and belongs to this Core, `core_personality` is read read-only. The traits `sensitivity`, `empathy`, and `openness` (when present, each clamped to −1..=1) are averaged:

```text
personality_factor = clamp(1.0 + 0.5 * average(present_traits), 0.5, 1.5)
```

Missing/unreadable/foreign personality yields a factor of `1.0`. Personality is never written and never grants authority.

**Finalization.** Dimensions are clamped into range, intensity is recomputed deterministically:

```text
intensity = clamp(0.4 * |valence| + 0.6 * mean(arousal, joy, calm, curiosity, tension), 0.0, 1.0)
```

The new state is written to `emotion_state`, an append-only row is added to `emotion_history`, and `revision` is incremented.

## 5. Continuity, activities, routines, outfits

* `update_last_active(session_id)` / `update_last_active_at(session_id, now)` set the last-active timestamp and optional session id.
* `continuity_summary()` / `continuity_summary_at(now)` compute elapsed offline seconds (never negative, capped at 10 years) and assemble bounded recent experiences (≤ 8), open threads (≤ 8), and activities (≤ 8).
* `record_experience(summary)` appends a bounded experience event.
* `open_thread(summary)` / `resolve_thread(id)` manage continuity threads.
* `upsert_activity(activity)` / `list_activities(limit)` manage activities.
* `set_routine(routine)` / `list_routines(limit)` manage routines.
* `record_outfit(record)` / `outfit_decisions(limit)` manage append-only outfit decisions.
* `emotion_history(limit)` returns append-only emotion snapshots newest-first (limit capped at 1000).

`CoreSdkContext` now also opens and verifies `PersonalStore` for the same Core and exposes:

```rust
pub fn personal(&self) -> Result<&PersonalStore, SdkError>
```

The existing `identity()`, `capabilities()`, `memory()`, `personality()`, and `refresh_personality()` APIs are unchanged. `personal` is also a public field, matching the existing `memory` field style.

## 6. Migration

Migration is **new-file additive**; no existing database is modified.

* A Core without `personal.db` gets a new database on first open.
* The `personal_schema` marker records schema `aegis.core.personal.v1` and revision `1`.
* Re-opening an existing database only runs `CREATE ... IF NOT EXISTS`; triggers are idempotent.
* A future higher schema revision is rejected (`CorruptData("personal schema is unsupported")`); there is no in-place destructive migration.
* A pre-existing `personal.db` without exactly one valid ownership row is rejected (fail-closed).

## 7. Backup integration

Milestone 1's backup inventory already lists `personal.db` as an optional sidecar with ownership table `personal_ownership` and requires exactly one row whose `core_uuid` matches the Core. This store produces exactly that shape, plus `database_instance_uuid` and immutable triggers. Backups therefore discover and validate the new database without any M1 change. This store additionally verifies the database instance UUID that M1 records for `core.db`.

## 8. Tests and results

Commands run (only the two approved packages):

```text
cargo test -p aegis-contracts -p aegis-core-sdk
cargo clippy -p aegis-core-sdk -p aegis-contracts --all-targets -- -D warnings
```

Results:

* `aegis-contracts` unit tests: **25 passed** (20 pre-existing + 5 new personal-contract tests).
* `aegis-core-sdk` unit tests: **0**.
* `aegis-core-sdk` `tests/personal.rs`: **12 passed**.
* `aegis-core-sdk` `tests/sdk.rs`: **14 passed** (pre-existing, unchanged).
* Total: **51 passed, 0 failed**.
* Clippy: **clean** with `-D warnings`.

New personal tests:

1. `personal_store_binds_identity_and_opens_database` — path/identity/app marker/journal/ownership row.
2. `two_cores_keep_distinct_emotions_and_history` — per-Core isolation of emotions, history, experiences, threads.
3. `personal_state_persists_across_reopen` — state/history/continuity survive reopen.
4. `elapsed_offline_is_computed_and_bounded` — elapsed calculation, negative clamp, 10-year cap.
5. `ownership_and_instance_mismatch_are_rejected` — copied DB and wrong instance UUID both rejected.
6. `emotion_decay_and_multiple_influences_are_deterministic` — weighted multi-influence, identical results on two Cores, exact half-life decay.
7. `emotion_history_and_append_only_tables_reject_mutation` — triggers block mutation, history bounded.
8. `continuity_and_listings_are_bounded` — 8/8/8 summary caps, long inputs rejected, thread resolution.
9. `outfits_and_routines_round_trip` — append-only outfits, routine upsert.
10. `personality_modulates_influence_weight` — bounded personality trait modulation.
11. `core_sdk_context_integrates_personal_store` — context opens/persists personal state.
12. `history_entry_shape_is_stable` — stable history snapshot shape.

## 9. Limitations

* `PersonalContractError` is mapped onto the existing `SdkError` variants (`InvalidMemory` for caller input, `CorruptData` for stored data) because M3 was not permitted to add a new `SdkError` variant.
* Personality modulation reads `memory.db` read-only on each emotion update. If the file is absent, not owned by this Core, or missing `core_personality`, the factor is `1.0`; personality is never modified.
* A live backup must not run while another process writes `personal.db`; like the other sidecars, raw file copies are not transactionally consistent if the database is being written at the same moment.
* Very long offline periods decay all dimensions to zero by floating-point underflow; this is the intended neutral state.
* `open_thread` identifiers use UUID v4 (randomness is limited to opaque record ids; emotion generation remains fully deterministic).
* Personal state is not yet wired into CLI/GUI/lifecycle orchestration beyond `CoreSdkContext`; this milestone defines and verifies the storage/engine layer only.
* All state is descriptive; none of it is an authorization input.

## 10. Next steps

* Surface `CoreSdkContext::personal()` in conversation/cognition context assembly (bounded summaries only).
* Add lifecycle/CLI reporting for personal-state presence and schema revision.
* Consider future additive schema revisions (e.g. richer activity metadata) behind `PERSONAL_SCHEMA_REVISION`.
* Add optional export/import for personal state if cross-device continuity is required; keep ownership binding and append-only history intact.
