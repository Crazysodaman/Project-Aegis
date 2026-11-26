# Batch 5 — Agent 05: TUI & Diagnostics

Baseline: `2bd2513`. Work is additive; no commit, reset, rebase, or destructive git was
run. All pre-existing uncommitted work was preserved.

## Scope and constraints

- Ownership respected: only `crates/aegis-ui/src/model.rs`,
  `crates/aegis-ui/src/tui.rs`, `crates/aegis-ui/tests/tui_dashboard.rs`, and this
  document were changed.
- `gui.rs`, `html.rs`, `actions.rs`, `lib.rs`, `Cargo.toml`, `Cargo.lock`, the
  workspace manifest, `aegis-cli`, and every other crate are untouched.
- No dependencies were added. The renderer and model use only the crates already in
  `crates/aegis-ui/Cargo.toml`: `aegis-contracts`, `aegis-core-install`,
  `aegis-core-registry`, `aegis-core-runtime`, `aegis-diagnostics`, `aegis-modules`,
  `aegis-platform`, `aegis-safe`, `aegis-storage`, `chrono`, `serde_json`, `thiserror`.
- No mocked or fabricated operational values exist in production code. A section is
  either populated from a real backend read or rendered as `UNAVAILABLE`/`NONE`.

## Audit of existing code

- `model.rs` already read real Platform status, Core installer/registry entries,
  Core runtime status, module records, and the diagnostics report. It exposed
  `pub(crate)` views (`RuntimeView`, `IntegrityView`, `CoreView`, `ModulesView`) and
  the collectors used by the HTTP dashboard.
- `tui.rs` rendered Platform, Core roster, Modules, and Diagnostics only. It had no
  Fleet, GPU, inference, memory, service, or explicit errors/warnings sections, and no
  status classification.
- `tests/gui_dashboard.rs` had one terminal test checking IDs and ANSI colors.
- `aegis-fleet` (hosts, GPU telemetry), `aegis-service` (service state), and
  `aegis-cognition` (inference) are separate workspace crates that are **not**
  dependencies of `aegis-ui`, so their operational data cannot be read without
  changing `Cargo.toml`.

## Changes

### `crates/aegis-ui/src/model.rs`

- Added `DashboardModel`, a single real-data aggregate with sections for:
  platform status + platform error, installed Cores, modules, diagnostics, database,
  Fleet, GPU, active inference, service, memory, errors, and warnings.
- Added `DashboardModel::collect(root)`, which populates every section reachable from
  existing dependencies and leaves unlinked sections as `None`.
- Added database status derived from real files: `StoragePaths` file sizes for
  `platform.db`/`safe.db` and the sum of per-Core `database_bytes` from diagnostics.
- Added explicit optional view types mirroring real upstream fields so a future caller
  can populate them without inventing data: `FleetView`/`FleetHostView`, `GpuView`,
  `InferenceView`, `ServiceView`, `MemoryView`.
- Added `StatusClass` (`Ok`/`Warn`/`Error`) and pure classifiers:
  - `classify_platform` — platform availability, STOP state, diagnostics overall.
  - `classify_core` — integrity first, then runtime state/heartbeat freshness.
  - `classify_service` — error, running + pid alive + socket reachable.
  - `classify_gpu` — unavailable vs. temperature/utilization thresholds.
  - `classify_fleet` — empty, partial reachability, or total outage.
  - `classify_database` — error, platform DB present, or unknown.
- Wired errors/warnings from real values: diagnostics reasons, Core runtime
  `last_error`, module registry state, missing platform DB, and unlinked data sources.

### `crates/aegis-ui/src/tui.rs`

- Kept `render_dashboard`, `run_tui_once`, and `run_tui` signatures unchanged for
  backward compatibility. `render_dashboard` now builds `DashboardModel::collect`
  and delegates to a new `pub(crate) render_model`.
- Renders every required section with a status badge and honest labels:
  - Platform (identity, zero-Core, core count, STOP, authority, diagnostics state).
  - Installed Cores (lifecycle, runtime, heartbeat, PID, integrity, runtime error,
    per-Core status badge).
  - Fleet hosts (`UNAVAILABLE` when no data source is linked; `NONE` when empty).
  - GPU (`UNAVAILABLE` or telemetry: name, utilization, free VRAM, temperature).
  - Active inference (`UNAVAILABLE`/`NONE`/real model + host).
  - Memory (`UNAVAILABLE`/`NONE`/used of total bytes and percent).
  - Database (real file sizes or `UNAVAILABLE`).
  - Service (`UNAVAILABLE`/running, PID, process, socket).
  - Modules (registry state and records).
  - Errors and warnings, each with `ERROR:`/`WARN:` labels or `NONE`.
- All ANSI colors and the `STOP` marker used by the existing test are preserved.

### `crates/aegis-ui/tests/tui_dashboard.rs` (new)

Integration coverage over real temp Platform fixtures and an uninitialized root.

## Verification

- `cargo test -p aegis-ui` — **29 passed, 0 failed**
  - 12 lib unit tests (6 model classification, 6 TUI rendering/helpers)
  - 14 existing `gui_dashboard.rs` tests (unchanged, still passing)
  - 3 new `tui_dashboard.rs` integration tests
- `cargo clippy -p aegis-ui --all-targets -- -D warnings` — **clean, no warnings**
- No whole-workspace gates were run.

## Wired vs. unavailable data

Wired from real backend structures:

| Section | Source |
| --- | --- |
| Platform health, identity, STOP, core count | `aegis_platform::PlatformService::status` (`PlatformStatus`) |
| Diagnostics overall + reasons | `aegis_diagnostics::Diagnostics::collect` (`DiagnosticsReport`) |
| Installed Cores + lifecycle | `aegis_core_install::CoreInstaller::list` (`CoreRegistryEntry`) |
| Core runtime | `aegis_core_runtime::CoreRuntime::list` (`RuntimeStatus`) |
| Core integrity | `CoreInstaller::inspect` (`IntegrityReport`) |
| Modules | `aegis_modules::ModuleRegistry::list` (`ModuleRecord`) |
| Database status | `aegis_storage::StoragePaths` sizes + diagnostics `database_bytes` |
| Errors/warnings | Diagnostics reasons, runtime `last_error`, registry state |

Explicitly unavailable (rendered as `UNAVAILABLE`, never fabricated):

| Section | Gap |
| --- | --- |
| Fleet hosts | `aegis-fleet` is not a dependency of `aegis-ui` |
| GPU availability | `aegis-fleet::gpu` is not a dependency |
| Active inference | `aegis-cognition` is not a dependency |
| Memory telemetry | No memory telemetry crate is a dependency |
| Service status | `aegis-service` is not a dependency |

## Limitations

- The console remains an honest snapshot that re-reads state on refresh; it is not a
  live event stream.
- `DashboardModel` and its views are crate-internal (`pub(crate)`), so external callers
  cannot yet inject Fleet/GPU/inference/service/memory data through the public API.
- `collect_status` calls `PlatformService::start`, which initializes Platform/module/safe
  stores if absent. This is pre-existing behavior, not introduced by this change, but
  it means rendering an uninitialized root can create database files.
- Classification thresholds for GPU temperature (>= 80 C) and utilization (>= 90%)
  are conservative local rules; they are not read from a Fleet policy because
  `aegis-fleet` is not linked.

## Next steps

1. With owner approval, add workspace dependencies `aegis-fleet`, `aegis-service`, and
   `aegis-cognition` (plus a memory telemetry source if one is designated) to
   `crates/aegis-ui/Cargo.toml`.
2. Populate `DashboardModel::collect` from `FleetStore::list_hosts`/GPU probe,
   `ServiceManager::status`, and cognition/placement status, mapping them into
   `FleetView`, `GpuView`, `ServiceView`, and `InferenceView`.
3. Optionally expose `DashboardModel` and `render_model` publicly so the GUI owner can
   render the same honest sections, and add a `/api/dashboard` JSON endpoint.
4. Add a live refresh/event source if operational streaming becomes a requirement.
