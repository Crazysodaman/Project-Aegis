# Aegis FINAL — Agent 12 (CLI & TUI)

HEAD: `2bd2513` baseline working tree.

## Files changed
- `crates/aegis-cli/src/main.rs` — new `core install-package` and `voice synthesize` commands plus usage lines.
- `crates/aegis-ui/src/model.rs` — optional `run/status/world.json` and `run/status/personal.json` loaders with shared freshness/validation, new `WorldView`/`PersonalView`, warnings.
- `crates/aegis-ui/src/tui.rs` — renders `World` and `Personal` sections (fresh values, `STALE`, or `UNAVAILABLE` with reasons).
- `crates/aegis-ui/tests/tui_dashboard.rs` — new tests for fresh, missing, stale, and malformed world/personal snapshots.
- `docs/implementation/AEGIS_FINAL_AGENT12_REPORT.md` — this report.

## Commands added
- `aegis core install-package <package-path> --root <path>` — validates a `.corepkg` manifest/directory via `aegis-core-install` `install_package` (Core number/name/UUID/digest checks) and installs only after validation. Manually smoke-tested with a Core #2 package; output is the normal `aegis.cli.core_installation.v1` payload.
- `aegis voice synthesize --root <path> --core <id> --text <text> --out <wav> [--rate <percent>] [--volume <percent>]` — uses the embodiment `SapiTtsProvider` (Windows `System.Speech`), writes a validated WAV, never plays audio. Requires owner proof before reading private per-Core voice data from `EmbodimentStore`. `--rate` 50–200, `--volume` 0–100. On non-Windows the provider returns a typed `SapiUnavailable` failure.
- `--help` usage now lists both commands. Existing commands/palette unchanged.

## TUI behavior
- Reads optional `run/status/world.json`: `local_time`, `timezone`, `time_of_day`, `season`, `condition`/`weather.condition`, `temperature_celsius`/`weather.temperature_celsius`.
- Reads optional `run/status/personal.json`: `core_uuid`, `emotion` (object dimensions or `emotion_summary`), `continuity` (`summary`/`recent_experiences` or `continuity_summary`), `activities`, `routines`.
- Snapshot is trusted only when fresh (≤60s by `updated_at`, else file mtime) and valid. Missing/stale/malformed data renders `UNAVAILABLE`/`STALE` with an explicit reason; nothing is fabricated.

## Verification
- `cargo test -p aegis-cli -p aegis-ui` — all pass:
  - `aegis-cli` integration: 23 passed.
  - `aegis-ui` unit: 22 passed.
  - `aegis-ui` `gui_dashboard`: 37 passed (pre-existing).
  - `aegis-ui` `tui_dashboard`: 13 passed (4 new world/personal tests).
- `cargo clippy -p aegis-cli -p aegis-ui --all-targets -- -D warnings` — clean.
- Manual smoke: `core install-package` installed a valid Core #2 package; `voice synthesize` produced an 85,126-byte validated WAV with `played: false` on this Windows host.

## Real vs unavailable / limitations
- Real: `.corepkg` install delegation, SAPI WAV synthesis, world/personal snapshot freshness/validation and TUI rendering.
- World/personal data is only as real as the optional operator-written snapshots; no CLI emitter exists yet.
- Voice has no playback and no speech-to-text/microphone work here.
- Native GUI is not in scope; `gui.rs`/`html.rs`/`actions.rs` were not modified.
