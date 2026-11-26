# Batch 8 — Agent M7: Distribution & Independent Internal QA (Milestone 7)

- **Date:** 2026-11-26
- **Baseline HEAD:** `2bd2513`
- **Role:** internal, adversarial QA pass (not independent external acceptance)
- **Workspace:** `D:\Aegis` (Windows, PowerShell)
- **Constraints honored:** no commit/reset/rebase/clean/deploy; no changes to
  `D:\SofiaDev\SofiaAdaLyra`; no production source edits; no `Cargo.toml` /
  `Cargo.lock` edits; no edits to existing tests; no new dependencies; all
  databases created under fresh `tempfile` roots; live `var/` never touched.
- **Preserved:** every pre-existing uncommitted Batch 3–8 change was left intact.

## 1. Files added (only these)

| File | Purpose |
| --- | --- |
| `crates/aegis-platform/tests/batch8_qa.rs` | Weather-failure honesty, freshness, DST clock |
| `crates/aegis-core-sdk/tests/batch8_qa.rs` | Shared world + distinct emotions, ownership rejection, continuity, identity |
| `crates/aegis-conversation/tests/batch8_qa.rs` | Restart/recovery, context prompts, flags |
| `crates/aegis-embodiment/tests/batch8_qa.rs` | Outfit/expression/voice divergence, safety, isolation |
| `crates/aegis-backup/tests/batch8_qa.rs` | End-to-end backup/verify/restore, identity, isolation |
| `docs/implementation/BATCH8_AGENT07_M7_QA.md` | This report |

Nothing else was added or modified. Verification: `git status` shows the five
test files as the only new Batch-8 QA artifacts; production files remain exactly
as the earlier agent milestones left them.

## 2. Commands and results

### Tests

```
cargo test -p aegis-platform -p aegis-core-sdk -p aegis-conversation \
           -p aegis-embodiment -p aegis-backup
```

| Package | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `aegis-platform` | 29 | 0 | 0 |
| `aegis-core-sdk` | 30 | 0 | 0 |
| `aegis-conversation` | 23 | 0 | 0 |
| `aegis-embodiment` | 108 | 0 | 1 (pre-existing live LAN) |
| `aegis-backup` | 20 | 0 | 0 |
| **Total (scoped)** | **210** | **0** | **1** |

New QA tests added by this pass: **16**, all passing.

| New suite | Tests |
| --- | ---: |
| `aegis-platform/tests/batch8_qa.rs` | 4 |
| `aegis-core-sdk/tests/batch8_qa.rs` | 4 |
| `aegis-conversation/tests/batch8_qa.rs` | 3 |
| `aegis-embodiment/tests/batch8_qa.rs` | 3 |
| `aegis-backup/tests/batch8_qa.rs` | 2 |

### Clippy

```
cargo clippy -p aegis-platform -p aegis-core-sdk -p aegis-conversation \
             -p aegis-embodiment -p aegis-backup --all-targets -- -D warnings
```

Result: **clean** (`Finished` with no warnings). Whole-workspace gates were not
run, per scope.

## 3. Verified scenario matrix

| # | Scenario | Test(s) | Crate | Result |
| --- | --- | --- | --- | --- |
| 1 | Multiple Cores share one world snapshot while keeping different personal emotions, both surviving reopen | `cores_share_one_world_snapshot_but_keep_distinct_emotions_across_reopen` | core-sdk | PASS |
| 2 | Cross-Core contamination rejected: A's `personal.db` copied into B | `copied_personal_database_is_rejected_for_another_core` | core-sdk | PASS |
| 3 | Continuity: fixed-past `last_active`, exact `elapsed_offline`, bounded experiences/threads/activities across reopen; negative clock clamps to 0 | `continuity_elapsed_offline_is_exact_across_reopen_and_bounded` | core-sdk | PASS |
| 4 | Restart/recovery of `WorldService` + `PersonalStore` + conversation session; messages, continuity, and world snapshot persist without loss | `restart_recovery_preserves_world_personal_and_conversation_state` | conversation | PASS |
| 5 | Failing `WeatherProvider` yields no invented weather + honest freshness, clock/Platform still work; stale recomputation is age-based | `failing_weather_provider_is_honest_and_platform_still_works`, `weather_freshness_is_recomputed_from_snapshot_age` | platform | PASS |
| 6 | America/Chicago DST spring-forward and fall-back at fixed instants: offsets, repeated/skipped local hour, date, season | `chicago_spring_forward_dst_offsets_are_correct`, `chicago_fall_back_dst_offsets_and_repeated_hour_are_correct` | platform | PASS |
| 7 | Same world + different personality/emotion ⇒ distinct bounded prompts; context flags accurate; unavailable world still valid | `same_world_with_distinct_personality_and_emotion_yields_distinct_bounded_prompts`, `unavailable_world_and_default_options_remain_valid_and_honest` | conversation | PASS |
| 8 | Same world/activity, different wardrobe/emotion ⇒ different outfit/expression/voice; weather safety + identity; no cross-Core leakage | `same_world_different_wardrobe_and_emotion_change_presentation`, `weather_safety_identity_and_cross_core_constraints_are_enforced`, `voice_is_bounded_for_nonfinite_emotion_and_expressions_are_never_invented` | embodiment | PASS |
| 9 | Install Core, create real memory/personal/conversation rows, backup, verify, restore into fresh temp root; ownership/data survive; A cannot read as B; contaminated tree rejected | `end_to_end_backup_restore_preserves_data_and_isolates_cores` | backup | PASS |
| 10 | Installed Core UUID/number/display name/instance/created-at unchanged after restart and after restore; restored root re-backs up identically | `core_identity_is_unchanged_across_sdk_reopen_and_world_start`, `restored_identity_is_unchanged_and_backup_is_reproducible` | core-sdk, backup | PASS |

Additional properties directly asserted:

- Shared snapshot: two independent `snapshot_for_core` reads are identical, and
  the deterministic `latest_at` view equals the generated snapshot.
- Emotion isolation: same fixed instant, Core A and Core B end with different
  six-dimensional states; states differ after reopen too.
- Ownership: cross-Core `personal.db` copy returns `SdkError::OwnershipMismatch`;
  cross-Core embodiment select/render returns `EmbodimentError::OwnershipMismatch`.
- Honesty: unavailable weather has `source = "unavailable"`, a bounded reason,
  and stays `Unavailable` regardless of age; no weather/forecast/sun invented.
- Bounds: continuity lists capped at 8/8/8; prompts ≤ `MAX_SYSTEM_PROMPT_CHARS`;
  voice rate/volume clamped even for NaN/∞ emotion dimensions; absent expression
  names are never invented.
- Backup: v2 manifest records memory/personal/conversation DBs; `verify_backup`
  is `Verified`; `CoreInstaller::verify` passes for every restored Core; a
  post-restore cross-Core DB swap makes the next backup fail closed with no
  manifest written.
- Identity: Core A stays `core_number = 1`, `display_name = "Core A"`, same
  `core_uuid` and `database_instance_uuid`, same `created_at` across restart,
  restore, and re-backup. Roster was not modified.

## 4. Unverified / blocked (explicitly NOT claimed)

These were unavailable or out of safe scope and are **not** exercised as
acceptance evidence:

1. **Artemis** — no Artemis component, endpoint, or credential was used or
   available.
2. **Multi-machine / cross-host** — all tests run on a single Windows host with
   temp directories. Same-host loopback LAN tests exist elsewhere but no real
   multi-machine distribution, discovery, or remote restore was performed.
3. **Live gameplay** — no interactive game session, real user, or live Core loop
   was run.
4. **Owner acceptance / independent external QA** — no human owner sign-off, no
   independent external reviewer, and no third-party acceptance was performed.
5. **Live weather network correctness** — this pass validated the provider
   *boundary* (failure honesty) and snapshot freshness. The M6 env-gated live
   Open-Meteo test is skipped/not asserted here; no network acceptance claim.
6. **GUI/native rendering, SAPI consumption of `VoiceProjection`** — the
   presentation layer is verified as data + headless BMP only.
7. **Cross-machine restore of `owner.proof`** — DPAPI binds it to the current
   Windows user/machine; bytes are preserved but cross-machine decryption was
   not tested.
8. **Concurrent-write backup safety** — backup correctly refuses running Cores;
   behavior under an actively-writing database was intentionally not exercised.

## 5. Defects found

**None.** No product defect was observed in the scoped Batch 8 scenarios; all
16 new adversarial tests pass and clippy is clean. No source was changed.

Observations that are documented limitations rather than defects:

- `aegis-conversation` has no direct `chrono` dependency, so the restart test
  uses `WorldService::snapshot`/`shared_snapshot` rather than a fixed-instant
  call; freshness equality is still exact because generation and read are
  milliseconds apart.
- `aegis-backup` has no dependency on the SDK/conversation crates, so its test
  seeds product-shaped SQLite databases (identical ownership and data table
  schemas) instead of calling `PersonalStore`/`ConversationEngine`. Product-API
  personal and conversation persistence is covered in the core-sdk and
  conversation suites.
- Freshness is snapshot-age based, not per-observation-age based; cached
  observations within the provider TTL can appear under a `Fresh` snapshot. This
  is the documented M2/M6 design, not a regression.

## 6. Residual risks

- **Backup authenticity:** backups provide content integrity, not signatures. A
  party that rewrites both a database and the manifest can forge a
  self-consistent pair; ownership checks catch naive cross-Core swaps but not a
  full adversarial forgery.
- **Quiescence requirement:** raw SQLite copies are only valid when no process
  is writing; WAL sidecars are excluded by design.
- **Weather transport:** the live provider shells out to `curl.exe` and depends
  on network/Open-Meteo availability; offline behavior is honest (`Unavailable`)
  but not a live-data guarantee.
- **Heuristic presentation:** wardrobe safety and expression choice are
  keyword/score heuristics; unusual asset names could be misclassified.
- **Coverage breadth:** one IANA zone (`America/Chicago`) is verified for DST;
  other zones/aliases are not exhaustively sampled.
- **Human factors:** no subjective quality, UX, or owner-preference evaluation
  was performed.

## 7. Acceptance recommendation

**Internal QA recommendation: ACCEPT for the internal Batch 8 scope.**

All ten required internal scenarios pass (16 new adversarial tests), the five
scoped packages report 210 passing / 0 failing (plus one pre-existing ignored
live-LAN test), and scoped clippy with `-D warnings` is clean. State isolation,
ownership rejection, Bounded prompts/continuity, honest weather failure, DST
correctness, deterministic embodiment, and backup/restore integrity/identity are
all evidenced.

**This is NOT independent external acceptance.** It does not substitute for:

- Artemis integration acceptance,
- real multi-machine distribution/restore acceptance,
- live gameplay and long-run stability acceptance,
- owner/human acceptance, or
- a third-party independent QA review.

Those remain **UNAVAILABLE/BLOCKED** and must be completed by an independent
external party before any release or "accepted" claim. The correct status is:
*internal adversarial QA passed; independent external acceptance pending.*
