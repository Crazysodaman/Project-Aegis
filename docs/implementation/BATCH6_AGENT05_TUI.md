# Batch 6 — Agent 05: TUI & Platform Diagnostics

Baseline: `2bd2513`. Work is additive; no commit, deploy, reset, rebase, clean, or
destructive git was run. All pre-existing uncommitted work was preserved.

## Scope and constraints

- Edited only:
  - `crates/aegis-ui/src/model.rs`
  - `crates/aegis-ui/src/tui.rs`
  - `crates/aegis-ui/src/lib.rs`
  - `crates/aegis-ui/tests/tui_dashboard.rs`
  - `docs/implementation/BATCH6_AGENT05_TUI.md` (this document)
- Not edited: `crates/aegis-ui/src/gui.rs`, `html.rs`, `actions.rs`, `crates/aegis-cli`,
  the workspace `Cargo.toml`, `Cargo.lock`, `.gitignore`, `docs/completion`, or any
  other crate.
- No dependency was added, including workspace crates. Only the existing
  `serde_json`, `aegis-*`, `chrono`, and `thiserror` dependencies are used.
- No mocked or fabricated operational values were introduced. Sections without a
  linked data source remain `UNAVAILABLE` with an explicit reason.

## Audit of Batch 5 state

- `model.rs` built a real `DashboardModel` from Platform status, Core installer
  entries, Core runtime, module records, and diagnostics, and exposed pure
  `classify_*` helpers plus `StatusClass`.
- `tui.rs` rendered Platform, Cores, Fleet, GPU, inference, memory, database,
  service, modules, and an errors/warnings block. It kept `render_dashboard`,
  `run_tui`, and `run_tui_once`.
- `tests/tui_dashboard.rs` had 3 integration tests over real temp Platform fixtures.
- There was no machine-readable snapshot, no severity-classified issue list, and no
  contradiction detection. Per-Core runtime errors were appended to `model.errors`
  while integrity and heartbeat problems were only visible indirectly through the
  status badge.

## Changes

### `crates/aegis-ui/src/model.rs`

- Added stable unavailable-reason constants shared by the TUI and JSON summary:
  `FLEET_UNAVAILABLE_REASON`, `GPU_UNAVAILABLE_REASON`,
  `INFERENCE_UNAVAILABLE_REASON`, `SERVICE_UNAVAILABLE_REASON`,
  `MEMORY_UNAVAILABLE_REASON`.
- Extended `StatusClass` with a machine-readable `as_str()` (`ok`/`warn`/`error`)
  alongside the existing display `label()`.
- Added `Severity` (`Error`/`Warn`) and `Issue { severity, code, message }`.
- Added `core_issues(cores)`:
  - integrity not `verified` → `integrity_failure` error;
  - runtime `last_error` → `runtime_error` error;
  - runtime state `failed` → `runtime_failed` error.
- Added `consistency_warnings(model)`, all `Warn` severity:
  - `stop_active` — STOP is active;
  - `zero_core_contradiction` — Platform says zero Cores but Cores are registered;
  - `core_count_mismatch` — Platform count differs from the registered roster;
  - `no_cores` — no Cores registered;
  - `duplicate_core_number`;
  - `heartbeat_not_fresh` — active Core (`running`/`starting`/`stopping`) with a
    stale or absent heartbeat;
  - `stop_with_active_core` — STOP active while a Core runtime is active.
- Added `issues(model)`, combining caller-supplied errors/warnings, real Core
  issues, and consistency contradictions, each with a severity and stable code.
- Removed the earlier ad-hoc Core `last_error` push from `collect`, since
  `core_issues` now owns runtime error surfacing (avoids duplication).
- Added `overall_class(model)`, worst-case operational health from Platform,
  Cores, database, and only the optional sections that are actually populated
  (unavailable data sources do not degrade health).
- Added `summary_json(&DashboardModel) -> serde_json::Value`, schema
  `aegis.ui.summary.v1`, with:
  - `generated_at`, `overall_health`;
  - `platform`, `cores`, `modules`, `diagnostics`, `database`, `fleet`, `gpu`,
    `inference`, `service`, `memory`;
  - every section carries `status` (`available`/`none`/`unavailable`), and every
    unavailable section carries an explicit `unavailable_reason`;
  - `issues.errors` / `issues.warnings` with `{severity, code, message}`;
  - `consistency` warnings.
- Added public `render_summary_json(root)`, which collects a real snapshot and
  returns the same value the TUI/diagnostics can share.

### `crates/aegis-ui/src/tui.rs`

- Kept `render_dashboard`, `run_tui`, and `run_tui_once` signatures and behavior.
- Reused the shared unavailable-reason constants so console text and JSON agree.
- `render_issues` now consumes `model::issues`, splitting by `Severity`:
  `ERROR: <message> (code)` and `WARN: <message> (code)`, or `NONE`.
  This surfaces stale/absent heartbeats, integrity failures, runtime failures,
  STOP/zero-Core/count contradictions, and the existing backend warnings.

### `crates/aegis-ui/src/lib.rs`

- Re-exports exactly one new public item: `render_summary_json`. Existing exports
  (`render_dashboard`, `run_tui`, `run_tui_once`, dashboard server types) unchanged.

### `crates/aegis-ui/tests/tui_dashboard.rs`

- Added 2 integration tests over real temp Platform fixtures:
  - `summary_json_exposes_shared_values_and_unavailable_reasons`
  - `summary_json_flags_uninitialized_root_as_unavailable_with_consistency`
- Existing 3 tests unchanged.

## Verification

- `cargo test -p aegis-ui` — **48 passed, 0 failed**
  - 17 lib unit tests (6 model classification + 5 new summary/consistency/core
    issue tests + 6 TUI rendering/helper tests)
  - 26 existing `gui_dashboard.rs` tests (unchanged, still passing)
  - 5 `tui_dashboard.rs` integration tests (3 existing + 2 new)
- `cargo clippy -p aegis-ui --all-targets -- -D warnings` — **clean, no warnings**
- No whole-workspace gates were run.

## Wired vs. unavailable

Wired from real backend structures:

| Section | Source |
| --- | --- |
| Platform health, identity, STOP, core count | `aegis_platform::PlatformService::status` |
| Diagnostics overall + reasons | `aegis_diagnostics::Diagnostics::collect` |
| Installed Cores + lifecycle | `aegis_core_install::CoreInstaller::list` |
| Core runtime + heartbeat | `aegis_core_runtime::CoreRuntime::list` |
| Core integrity | `CoreInstaller::inspect` |
| Modules | `aegis_modules::ModuleRegistry::list` |
| Database status | `aegis_storage::StoragePaths` sizes + diagnostics `database_bytes` |
| Errors/warnings/consistency | Real Core/platform/module/diagnostics fields |

Explicitly unavailable (`status: "unavailable"`, never fabricated; still blocked
without an owner-approved dependency change):

| Section | Gap |
| --- | --- |
| Fleet hosts | `aegis-fleet` is not a dependency of `aegis-ui` |
| GPU telemetry | `aegis-fleet::gpu` is not a dependency |
| Active inference | `aegis-cognition` is not a dependency |
| Service status | `aegis-service` is not a dependency |
| Memory telemetry | No memory telemetry crate is a dependency |

## Limitations

- `summary_json`/`render_summary_json` reflect only the data `aegis-ui` can read.
  Fleet/GPU/inference/service/memory remain `unavailable` until dependencies are
  approved.
- `overall_health` intentionally ignores unavailable sections so missing data does
  not masquerade as degraded health; per-section status and `unavailable_reason`
  carry data completeness.
- Consistency checks are heuristics over the model (counts, active+heartbeat, STOP,
  duplicate numbers); they do not query upstream crates.
- The console is still an honest snapshot that re-reads state on refresh, not a live
  event stream.
- `collect_status` still calls `PlatformService::start` (pre-existing behavior), so
  rendering an uninitialized root may create database files.

## Next steps

1. With owner approval, add `aegis-fleet`, `aegis-service`, and `aegis-cognition`
   (plus a memory telemetry source if designated) to `crates/aegis-ui/Cargo.toml`.
2. Populate the corresponding model views and summary sections from those sources;
   they will automatically move from `unavailable` to `available`.
3. Optionally expose `summary_json` (model-based) publicly and add a `/api/summary`
   JSON endpoint for the HTTP dashboard, reusing the same snapshot.
4. Consider a live refresh/event source if operational streaming becomes a
   requirement.
