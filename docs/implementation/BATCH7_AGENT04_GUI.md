# Batch 7 — Agent 04: GUI External Status Snapshots

Baseline: `2bd2513`. All pre-existing uncommitted work was preserved. No commit,
deploy, reset, rebase, clean, or destructive git was run. `D:\SofiaDev\SofiaAdaLyra`
was not touched.

## Status

Complete for the in-scope HTTP dashboard work. The GUI now reads the same optional
Fleet/GPU/inference/service status snapshots as the Batch 7 TUI
(`docs/implementation/BATCH7_AGENT05_TUI.md`) and renders them on both the
dashboard index and the per-Core workspace. It also exposes the snapshot states
through `GET /api/status`.

This is still a **loopback HTTP dashboard**, not a native application window.
A native window remains **BLOCKED** without an owner-approved GUI toolkit; exact
dependency proposals are listed below. No dependency was added or changed.

## Exact files changed

| File | Change |
| --- | --- |
| `crates/aegis-ui/src/gui.rs` | Reuses `model::DashboardModel` snapshot state; adds `SectionStatus::Stale`; builds Fleet/GPU/Inference/Service sections; injects them into the index; appends them to the Core workspace; augments `GET /api/status` with bounded snapshot states. |
| `crates/aegis-ui/src/html.rs` | Added `render_sections`, a `__AEGIS_STATUS_SECTIONS__` index placeholder, consistent `/api/status` navigation links, and `STALE` badge/label handling. |
| `crates/aegis-ui/tests/gui_dashboard.rs` | Appended fresh/stale/malformed/missing/API-shape tests and updated the exact Core section-title expectation for the additive sections. |
| `docs/implementation/BATCH7_AGENT04_GUI.md` | This report (new). |

`crates/aegis-ui/src/actions.rs` was reviewed but not edited: mutation handling is
unchanged and already token-gated with bounded fields and secret redaction.
`model.rs`, `tui.rs`, and `lib.rs` were not edited (Agent 05 owns them).
`crates/aegis-cli`, workspace files, `Cargo.toml`, `Cargo.lock`, and every other
crate were not edited. `serde_json` was already a dependency of `aegis-ui`.

## Snapshot source and honesty

Snapshots are read from the Aegis root exactly as documented by Agent 05:

```
<root>/run/status/fleet.json
<root>/run/status/gpu.json
<root>/run/status/inference.json
<root>/run/status/service.json
```

The GUI reuses `model::DashboardModel`, which is the same loader the TUI uses.
A section is populated **only** when its snapshot exists, parses, validates, and
is fresh (≤60s by `updated_at`, else file mtime). All other cases render an
explicit reason:

| Snapshot condition | Dashboard / workspace status | API `state` |
| --- | --- | --- |
| File missing | `UNAVAILABLE: <reason>` | `unavailable` |
| Malformed JSON / invalid schema / unreadable | `UNAVAILABLE: <reason>` | `unavailable` |
| Older than 60s | `STALE: <reason>` | `stale` |
| Fresh and valid | values rendered, `AVAILABLE` | `fresh` |
| Fresh GPU `available:false` | `UNAVAILABLE: <error>` | `fresh` (data reports unavailable) |

No success is inferred from absent data. There are no dead controls and no
fabricated values.

## `GET /api/status`

The existing endpoint remains backward compatible: it still returns the real
platform status fields (`callsign`, `platform_id`, `authority`, `zero_core`,
`core_count`, `stop_active`, `generated_at`, ...). It additionally carries a
bounded `snapshots` object:

```json
{
  "callsign": "...",
  "core_count": 1,
  "zero_core": false,
  "snapshots": {
    "schema": "aegis.ui.status_snapshots.v1",
    "fleet":     {"state": "fresh",       "reason": null},
    "gpu":       {"state": "stale",       "reason": "GPU status snapshot at run/status/gpu.json is stale (...)"},
    "inference": {"state": "unavailable", "reason": "aegis-cognition is not a dependency of aegis-ui"},
    "service":   {"state": "unavailable", "reason": "..."}
  }
}
```

`state` is exactly one of `fresh`, `stale`, `unavailable`. `reason` is `null` for
fresh snapshots and a bounded human-readable cause otherwise. JSON is produced by
`serde_json`, so malformed snapshot contents can never break response framing or
panic the server. If the platform status itself cannot be read the endpoint keeps
its previous `503 {"ok":false,"error":"status_unavailable"}` behavior.

## Rendering and navigation

- Dashboard index: the four sections are server-rendered at page load into the
  main grid via `__AEGIS_STATUS_SECTIONS__`. They use the same real data and the
  same `STALE`/`UNAVAILABLE` reasons as the workspace.
- Core workspace: the same four sections are appended after `Activity`, so the
  HTML workspace and `GET /api/core/<uuid>` JSON stay in parity.
- Navigation: the index header and the workspace header both link to
  `/api/status`; the workspace keeps its breadcrumb, `JSON API`, and dashboard
  links. Existing routes (`/`, `/api/health`, `/api/status`, `/api/cores`,
  `/api/diagnostics`, `/api/modules`, `/api/core/<uuid>`, `/core/<uuid>`,
  `POST /api/actions`) are unchanged in behavior.

## Tests and counts

Commands run (package-scoped only; whole-workspace gates were not run):

```powershell
cargo test -p aegis-ui
cargo clippy -p aegis-ui --all-targets -- -D warnings
```

`cargo test -p aegis-ui`: **62 passed, 0 failed, 0 ignored**.
- 22 crate unit tests (17 pre-existing + 5 snapshot loader tests from Agent 05).
- 31 `tests/gui_dashboard.rs` tests (26 pre-existing + 5 new):
  1. `dashboard_renders_fresh_status_snapshot_sections`
  2. `stale_status_snapshot_renders_stale_not_values`
  3. `malformed_status_snapshot_is_unavailable_without_panic`
  4. `missing_status_snapshots_are_unavailable`
  5. `status_api_exposes_snapshot_states_and_preserves_platform_fields`
- 9 `tests/tui_dashboard.rs` tests (unchanged).

All tests use temporary directories. `cargo clippy -p aegis-ui --all-targets --
-D warnings`: **clean, exit 0**.

## Wired vs. still unavailable

Wired from optional fresh snapshots (with explicit `STALE`/`UNAVAILABLE` fallback):

| Section | Source when fresh |
| --- | --- |
| Fleet hosts | `run/status/fleet.json` |
| GPU telemetry | `run/status/gpu.json` |
| Active inference | `run/status/inference.json` |
| Service status | `run/status/service.json` |

Still unavailable:

| Section | Gap |
| --- | --- |
| Memory telemetry | No memory data source or snapshot schema is defined yet. |
| Personality / Voice / Appearance | `aegis-core-profiles` / `aegis-embodiment` are not `aegis-ui` dependencies. |
| Per-Core permission grants | No per-Core permission read API is linked. |

## Limitations

- No CLI/agent writes `run/status/*.json` yet. Until one does, all four sections
  stay explicitly `UNAVAILABLE`.
- The index's 3-second JavaScript refresh updates Platform/Cores/Modules/
  Diagnostics only; snapshot sections are server-rendered at page load and update
  on reload. The API always reports the current states.
- A fresh GPU snapshot whose `available` field is `false` is still a `fresh`
  snapshot at the file level, but the GPU section renders `UNAVAILABLE` with the
  reported error rather than a healthy GPU.

## Native window blocker (explicit)

The surface is still a loopback HTTP dashboard, **not** a native OS window.
Producing an actual application window needs an owner-approved GUI toolkit and
edits this agent is not permitted to make. Exact proposal (none applied):

1. **Native widgets:** add `eframe` and `egui` (workspace-pinned versions) to
   `crates/aegis-ui/Cargo.toml`, add a binary entry point, and render the same
   real data in egui panels. Requires owner approval for two new external crates
   plus transitive dependencies and changes to workspace `Cargo.toml` /
   `Cargo.lock`.
2. **Native window wrapping the existing dashboard:** add `tao` and `wry`
   (WebView2 on Windows / WebKit elsewhere) and point the WebView at the
   in-process loopback dashboard. Requires owner approval, platform WebView
   runtime provisioning, and a WebView security review.
3. **Data completeness (separate approval):** add internal workspace crates
   `aegis-core-profiles`, `aegis-embodiment`, and/or `aegis-cognition` as
   `aegis-ui` dependencies to replace Memory/Personality/Voice/Appearance
   placeholders with real fields.

## Next steps

1. Owner decision on the native toolkit proposal; on approval add the dependency
   and an application entry point outside the restricted file set.
2. Add a real CLI/agent emitter that atomically writes the four schemas so the
   dashboard shows live values (never fabricated).
3. Optional: link each Fleet host / inference placement to a relevant Core
   workspace once the backend exposes that mapping.
4. Optional: include snapshot ages in `/api/status` for operator debugging.
