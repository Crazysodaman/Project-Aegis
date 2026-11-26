# Batch 6 Agent 04 — Native GUI Completion (HTTP Dashboard) Report

Baseline: `2bd2513`. This change is additive and preserves all uncommitted work.
No dependencies were added, no workspace files were edited, and no crate outside
`aegis-ui` was modified. `D:\SofiaDev\SofiaAdaLyra` was not touched.

## Status

Complete for the in-scope HTTP dashboard work. **A native application window is
BLOCKED**: the current surface is still a loopback HTTP dashboard rendered in a
browser, not a native OS window. Producing an actual application window requires
an owner-approved GUI toolkit dependency (proposal in "Native window blocker").

## Audit (before this batch)

- `crates/aegis-ui/src/gui.rs` served exact-match routes only: `/`,
  `/api/health`, `/api/status`, `/api/cores`, `/api/diagnostics`,
  `/api/modules`, `POST /api/actions`, plus a single prefix route
  `GET /core/<uuid>` from Batch 5.
- `crates/aegis-ui/src/html.rs` rendered the platform index and the per-Core
  workspace. The workspace already escaped user-controlled text and had one
  `dashboard` back link, but no breadcrumb and no link to a machine-readable
  form of the same workspace.
- No JSON endpoint mirrored the per-Core workspace; programmatic clients had to
  scrape HTML.
- The HTTP target/path length was not explicitly bounded beyond the 16 KiB
  header cap.
- Real data sources available as direct `aegis-ui` dependencies:
  `aegis-contracts`, `aegis-core-install`, `aegis-core-registry`,
  `aegis-core-runtime`, `aegis-diagnostics`, `aegis-modules`,
  `aegis-platform`, `aegis-safe`, `aegis-storage`.
- Not available (and therefore never fabricated): `aegis-core-profiles`
  (personality/voice/appearance), `aegis-cognition` (memory/inference),
  `aegis-embodiment` (appearance/avatar), and any per-Core permission API.

## Exact files changed

| File | Change |
| --- | --- |
| `crates/aegis-ui/src/gui.rs` | Added typed JSON parity (`CoreWorkspaceView::to_json`, `GET /api/core/<uuid>`), stable typed error codes, bounded JSON error detail, and a request-target byte bound. |
| `crates/aegis-ui/src/html.rs` | Added breadcrumb navigation, a JSON API link on the Core workspace, and header/breadcrumb styles. Existing HTML escaping retained. |
| `crates/aegis-ui/tests/gui_dashboard.rs` | Appended 7 integration tests (append-only; no existing tests changed). |
| `docs/implementation/BATCH6_AGENT04_GUI.md` | This report (new). |

`crates/aegis-ui/src/actions.rs` was reviewed but not edited: mutation handling
already had token gating, bounded fields, secret redaction, and typed JSON
errors. `model.rs`, `tui.rs`, `lib.rs`, `Cargo.toml`, `Cargo.lock`,
`.gitignore`, `docs/completion`, `crates/aegis-cli`, and all other crates were
not edited.

## JSON API parity

`GET /api/core/<core-uuid>` returns exactly the same real data as the HTML
workspace at `GET /core/<core-uuid>`.

- `200` with schema `aegis.ui.core_workspace.v1` when the Core is registered.
- `400` `{"ok":false,"error":"invalid_core_uuid"}` for a malformed identifier,
  including `/api/core` and `/api/core/`.
- `404` `{"ok":false,"error":"core_not_found"}` for a valid but unregistered
  UUID.
- `503` `{"ok":false,"error":"core_workspace_unavailable","detail":"..."}` when
  the local backend cannot be read. `detail` is the real backend message,
  bounded to 512 characters and safely JSON-encoded.

Response shape:

```json
{
  "ok": true,
  "schema": "aegis.ui.core_workspace.v1",
  "core_uuid": "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d",
  "core_number": 1,
  "display_name": "Core A",
  "lifecycle_state": "installed",
  "generated_at": "<rfc3339>",
  "sections": [
    {
      "title": "Identity",
      "status": "available",
      "note": null,
      "rows": [{"label": "Core UUID", "value": "..."}],
      "items": []
    }
  ]
}
```

Section `status` is one of `available`, `partial`, `unavailable`, matching the
HTML `data-status` values. `rows` is an ordered array of `{label, value}` so
ordering is preserved across HTML and JSON. No owner token or session token is
ever included.

## Navigation

- The index roster builds a `workspace` link for every Core pointing at
  `/core/<url-encoded-uuid>` (existing behavior, now covered by tests).
- The Core workspace now has a breadcrumb (`Dashboard / <uuid>`) and a
  `JSON API` link (`/api/core/<uuid>`) plus the `dashboard` back link.
- Unknown UUIDs render a readable `404` HTML page; malformed UUIDs render a
  readable `400` HTML page; backend failures render a readable `503` HTML page.
  Each carries the breadcrumb-relevant back navigation and never panics.

## Robustness

- HTML output escapes `&`, `<`, `>`, `"`, and `'` for every user-controlled
  string (display names, notes, row labels/values, list items, titles, UUIDs)
  in the workspace and error pages.
- JSON output is produced by `serde_json`, so text is safely escaped and cannot
  break the response framing.
- Request targets are bounded to 2048 bytes (`MAX_TARGET_BYTES`); oversized
  paths are rejected with `400` before routing.
- Existing bounds remain: 16 KiB headers, 100 headers, 64 KiB bodies, no
  `Transfer-Encoding`, 5 s socket timeouts, loopback-only bind.
- Malformed requests, oversized bodies, overlong targets, invalid UUIDs, and
  backend failures all return typed responses without panicking.

## Wired vs UNAVAILABLE sections

| Section | State | Source |
| --- | --- | --- |
| Identity | Wired | `CoreRegistry` entry + `CoreInstaller::inspect` identity, metadata, integrity. |
| Memory | UNAVAILABLE | No memory data source linked (`aegis-cognition` / Core memory tables are not dependencies). |
| Personality | UNAVAILABLE | `aegis-core-profiles` is not a dependency. |
| Voice | UNAVAILABLE | `aegis-embodiment` / `aegis-core-profiles` are not dependencies. |
| Appearance | UNAVAILABLE (theme) | No appearance/theme data source. The standard Aegis palette is applied to the dashboard only. |
| Modules | Wired (platform-wide) | `ModuleRegistry::list`; per-Core module bindings are not exposed by linked APIs. |
| Permissions | Partial | Platform-wide declared `ModuleRecord.permissions`; per-Core grants are UNAVAILABLE. |
| Diagnostics | Wired / Partial | `Diagnostics::collect` per-Core report plus `CoreRuntime::status` runtime fields. |
| Activity | Wired | Append-only `CoreRegistry::lifecycle_history` for the Core. |

Every unavailable or partial section names the missing data source in its note
in both HTML and JSON. There are no dead controls and no fabricated values.

## Tests and counts

Commands run (package-scoped only; whole-workspace gates were not run):

```powershell
cargo test -p aegis-ui
cargo clippy -p aegis-ui --all-targets -- -D warnings
```

`cargo test -p aegis-ui` result: **41 passed, 0 failed, 0 ignored**.

- 12 crate unit tests (unchanged).
- 26 `tests/gui_dashboard.rs` integration tests (19 existing + 7 new).
- 3 `tests/tui_dashboard.rs` tests (unchanged).

New integration tests appended:

1. `core_workspace_json_api_matches_html_workspace` — JSON fields, section
   titles/statuses, real identity rows, and parity with HTML.
2. `core_workspace_json_api_returns_typed_errors` — `404 core_not_found`,
   `400 invalid_core_uuid` for invalid/empty identifiers, and trailing-slash
   handling.
3. `core_workspace_json_api_reports_backend_failure_without_panic` — typed
   `503 core_workspace_unavailable` with bounded real detail.
4. `core_workspace_has_breadcrumb_json_link_and_roster_workspace_links` —
   roster link builder, breadcrumb, JSON API link, back link.
5. `core_workspace_paths_tolerate_trailing_slash` — HTML and JSON accept a
   trailing slash.
6. `core_workspace_json_api_serializes_user_text_safely` — JSON preserves the
   exact display name while HTML escapes `<script>`.
7. `overlong_request_target_is_rejected_without_panic` — 4096-byte path gets
   `400`, no panic.

`cargo clippy -p aegis-ui --all-targets -- -D warnings`: **clean, exit 0**.

## Native window blocker (explicit)

This is still a **loopback HTTP dashboard**, not a native application window.
The requirement for an "actual application window" is **BLOCKED** by the
no-new-dependency rule.

An actual native window cannot be produced with only the current dependencies
(`std::net` + server-rendered HTML). It needs a GUI toolkit. Exact proposals,
each requiring owner approval and edits to the workspace `Cargo.toml` and
`Cargo.lock`, which this agent is not permitted to make:

1. **Native widgets:** add `eframe` and `egui` (workspace-pinned versions) to
   `crates/aegis-ui/Cargo.toml`, add a binary entry point, and render the same
   real data in egui panels. Owner approval needed for two new external crates
   and their transitive dependencies.
2. **Native window wrapping the existing dashboard:** add `tao` (window/event
   loop) and `wry` (embedded WebView2/WebKit) and point the WebView at the
   in-process loopback dashboard. Owner approval needed for two new external
   crates, platform WebView runtime requirements, and a security review of
   enabling a WebView.
3. **Data completeness (separate approval):** add internal workspace crates
   `aegis-core-profiles`, `aegis-embodiment`, and/or `aegis-cognition` as
   `aegis-ui` dependencies to replace the Memory/Personality/Voice/Appearance
   UNAVAILABLE notes with real fields. Internal workspace crate additions still
   require owner approval and are not permitted in this batch.

None of these dependencies were added. Until an owner approves one, the native
window requirement remains BLOCKED and the dashboard remains HTTP-only.

## Next steps

1. Owner decision on the native toolkit proposal above; on approval, add the
   dependency and an application entry point outside the restricted file set.
2. Owner approval to link `aegis-core-profiles` / `aegis-embodiment` /
   `aegis-cognition` so Memory, Personality, Voice, and Appearance become real.
3. Owner approval for a Core permission/authorization read API so Permissions
   can move from Partial to a real per-Core view.
4. Optional: have the HTML workspace fetch `/api/core/<uuid>` for live refresh;
   today both surfaces independently read the same backend and therefore match.
