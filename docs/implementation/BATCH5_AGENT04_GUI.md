# Batch 5 Agent 04 — Native GUI (HTTP Dashboard) Report

Baseline: `2bd2513`. This change is additive. No dependencies were added, no
workspace files were edited, and no other crate was modified.

## Status

Complete. The Aegis UI remains a **loopback HTTP dashboard**, not a native
window. This batch adds a server-rendered per-Core workspace at
`GET /core/<core-uuid>` on top of the existing dashboard server.

## Audit (before changes)

- `crates/aegis-ui/src/gui.rs` served a fixed set of exact-match routes:
  `/`, `/api/health`, `/api/status`, `/api/cores`, `/api/diagnostics`,
  `/api/modules`, and `POST /api/actions`.
- `crates/aegis-ui/src/html.rs` held one self-contained `INDEX_HTML` page with
  the platform overview and roster. No per-Core page existed.
- The roster linked no Core workspace; no path-parameter routes existed.
- Real data APIs already available as direct `aegis-ui` dependencies:
  `aegis-contracts`, `aegis-core-install`, `aegis-core-registry`,
  `aegis-core-runtime`, `aegis-diagnostics`, `aegis-modules`,
  `aegis-platform`, `aegis-safe`, `aegis-storage`.
- Not available as dependencies (and therefore not fabricated):
  `aegis-core-profiles` (personality/voice/appearance), `aegis-cognition`
  (memory/inference), `aegis-embodiment` (appearance/avatar), and any Core
  memory or permission table API.

## Exact files changed

| File | Change |
| --- | --- |
| `crates/aegis-ui/src/gui.rs` | Added `GET /core/<uuid>` route, typed `WorkspaceError`, `CoreWorkspaceView`/`WorkspaceSection` models, `build_core_workspace`, `html_response`, and additive roster navigation support. |
| `crates/aegis-ui/src/html.rs` | Added HTML escaping, server-rendered Core workspace (required sections), error page, Core theme CSS variables, and a `workspace` link in the roster table. |
| `crates/aegis-ui/tests/gui_dashboard.rs` | Appended 5 integration tests. |
| `docs/implementation/BATCH5_AGENT04_GUI.md` | This report (new). |

No edits were made to `model.rs`, `tui.rs`, `lib.rs`, `Cargo.toml`,
`Cargo.lock`, `.gitignore`, docs/completion, or any other crate.

## Routes added

- `GET /core/<core-uuid>` — server-rendered Core workspace.
  - `200` when the Core is registered.
  - `400` for a malformed Core UUID.
  - `404` when the UUID is valid but not registered.
  - `503` when the backend storage/registry cannot be read.
  - All responses are `text/html; charset=utf-8`; unknown values are escaped.

## Wired vs UNAVAILABLE sections

| Section | State | Source |
| --- | --- | --- |
| Identity | Wired | `CoreRegistry` entry + `CoreInstaller::inspect` core identity, metadata, and integrity. |
| Memory | UNAVAILABLE | No memory data source linked (`aegis-cognition` / Core memory tables are not dependencies). |
| Personality | UNAVAILABLE | `aegis-core-profiles` is not a dependency. |
| Voice | UNAVAILABLE | `aegis-embodiment` / `aegis-core-profiles` are not dependencies. |
| Appearance | UNAVAILABLE (theme) | No appearance/theme data source. The standard Aegis palette is applied to the dashboard only. |
| Modules | Wired (platform-wide) | `ModuleRegistry::list`; per-Core module bindings are not provided by linked APIs. |
| Permissions | Partial | Declared platform-wide module permissions from `ModuleRecord.permissions`; per-Core grants are UNAVAILABLE. |
| Diagnostics | Wired / Partial | `Diagnostics::collect` per-Core report plus `CoreRuntime::status` runtime fields. |
| Activity | Wired | Append-only `CoreRegistry::lifecycle_history` for the Core. |

Every UNAVAILABLE section carries an explicit note naming the missing data
source. There are no dead buttons and no fabricated values.

## Theme handling

The workspace page defines the required palette consistently:
`--primary: #9400D3`, `--accent: #00C2FF`, `--tertiary: #39FF14`, and a black
background, reusing the existing dashboard palette. Because no per-Core theme
data source is linked, the Appearance section explicitly labels per-Core theme
data as UNAVAILABLE rather than inventing one.

## Error handling

- Mutations still require the session token and the `allow_mutations` flag.
- Core workspace errors are typed (`InvalidCoreId`, `NotFound`, `Backend`) and
  render safe HTML with no panics.
- All user-controlled text (display name, reasons, error strings, paths) is
  HTML-escaped (`&`, `<`, `>`, `"`, `'`).
- Owner and session tokens are never included in the Core workspace page.

## Tests

`cargo test -p aegis-ui` (34 tests total, all passing):

- 12 crate unit tests (pre-existing).
- 19 `tests/gui_dashboard.rs` integration tests (14 pre-existing + 5 new).
- 3 `tests/tui_dashboard.rs` tests (pre-existing, untouched).

New integration tests:

1. `core_workspace_renders_required_sections_with_real_backend_data` — all nine
   sections, real UUID/display name/lifecycle/integrity, no token leakage.
2. `unknown_core_workspace_returns_clear_html_error` — 404 for an unregistered
   UUID and 400 for a malformed UUID.
3. `core_workspace_escapes_user_controlled_text` — `<script>` display name is
   escaped in the rendered page.
4. `core_workspace_applies_core_theme_palette` — required palette colors and
   explicit UNAVAILABLE theme note.
5. `core_workspace_reports_backend_failure_without_panic` — 503 HTML error when
   the storage root is unusable.

## Verification

- `cargo test -p aegis-ui` — 34 passed, 0 failed.
- `cargo clippy -p aegis-ui --all-targets -- -D warnings` — clean.

Whole-workspace gates were not run, per instructions.

## Limitations

- This is still an HTTP dashboard, **not a native window**. There is no desktop
  windows, event loop, or native widget toolkit.
- Memory, Personality, Voice, per-Core Appearance/theme, and per-Core
  Permissions are UNAVAILABLE because their data crates are not dependencies of
  `aegis-ui`.
- Modules and Permissions are platform-wide; the linked APIs do not expose
  per-Core module bindings or Core permission grants.
- The route parser still accepts only exact HTTP/1.1 paths; the workspace route
  is a single safe prefix match with UUID validation.

## Next steps (require owner approval)

1. **Native window:** introduce an approved GUI toolkit dependency (for example
   a Rust window/webview crate) and an application entry point. This needs a
   dependency owner decision; do not add it unilaterally.
2. **Personality/Voice/Appearance:** add `aegis-core-profiles` and/or
   `aegis-embodiment` as `aegis-ui` dependencies once approved, then replace the
   UNAVAILABLE notes with real profile and theme fields.
3. **Memory:** add an approved Core memory/`aegis-cognition` data API and expose
   real memory health/usage.
4. **Permissions:** add a Core permission/authorization read API; until then the
   section remains Partial and platform-wide only.
5. **JSON parity:** add `GET /api/cores/<uuid>` mirroring the workspace view for
   programmatic clients, if desired.
