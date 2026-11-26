# AEGIS FINAL — Agent 18 (Installation, Windows & Debian Runtime)

HEAD audited: `2bd2513` (working tree also contains concurrent edits by other agents).
Scope touched: `crates/aegis-installer/**`, `crates/aegis-service/**` (tests only), this report.

## Files

- `crates/aegis-installer/src/preflight.rs` (new): read-only preflight for install / upgrade / uninstall plus data-only rollback plans.
  - Checks: absolute/redirected paths, best-effort writability, recorded version (downgrade/current/upgrade), backup destination (absolute, outside root, writable), owner proof, running service, callsign confirmation.
  - `mutations_performed: false` is invariant; no directory/file/database write, no `systemctl`/`sc.exe`.
- `crates/aegis-installer/src/lib.rs`: additive `pub mod preflight` + re-exports only; existing install/upgrade/uninstall unchanged.
- `crates/aegis-installer/tests/preflight.rs` (new): 10 tests.
- `crates/aegis-service/tests/service_units.rs` (new): 4 tests for systemd/Windows SCM unit generation and execution-plan rollback coverage.

## Audit summary

- Install: idempotent, preserves Platform identity, backfills a missing record, refuses an installed build newer than the current build.
- Upgrade: owner-authorized, refuses downgrades, same-version is a no-op, takes and verifies a backup before `PlatformService::initialize`; a backup destination inside the storage root is rejected by `aegis-backup` before any mutation (tested).
- Uninstall: preserve is the default; delete requires a verified backup, a callsign match, and no running service; redirected/reparse paths are rejected.
- Service definitions: systemd units and Windows SCM commands are rendered as inert data with validation and hardening; no OS service is installed or controlled by this build.

## Verification

- `cargo test -p aegis-installer -p aegis-service`: PASS, 85 tests total, 0 failed.
  - installer: 2 lib + 8 installation + 10 preflight = 20
  - service: 7 execution + 11 health + 6 logging + 15 registration + 5 runtime + 6 service + 4 service_units + 11 supervision = 65
- `cargo clippy -p aegis-installer -p aegis-service --all-targets --no-deps -- -D warnings`: PASS (clean) for both crates.
- Notable new tests: downgrade refusal, backup-gated upgrade + failed-backup leaves version unchanged, uninstall-preserve, delete needs backup+confirmation, running-service handling, rollback-plan validation, systemd/SCM unit generation.

## Debian / Linux runtime — UNVERIFIED

No Debian VM or systemd host was available. Linux unit *generation* is covered by compile-time tests only. Do not claim Debian support. Unverified Debian steps:

- Writing the rendered unit to `/etc/systemd/system/aegis-platform.service` and directory ownership/permissions.
- `systemctl daemon-reload`, `systemctl enable --now aegis-platform`, `systemctl is-active`, `systemctl disable --now` and `rm` rollback.
- Service-account/user selection and `/var/lib/aegis` ownership; `ProtectSystem=full` effect on the real storage root.
- Actual start/stop/restart and journald log capture under systemd.

Windows runtime was inspected at the code/test level only; no Windows SCM service was installed.

## Owner actions

- Stop the Platform service before an upgrade; the preflight now warns (does not block) because the upgrade path itself does not stop it, and a live SQLite copy is unsafe.
- Decide whether the CLI should require `preflight_upgrade`/`preflight_uninstall` before mutating calls (no CLI/workspace changes made here).
- On Debian: validate unit write + systemctl lifecycle manually in a VM, then record evidence before any support claim.
- Treat the permission check as best-effort filesystem metadata, not a real write probe (notably on Windows ACLs).
