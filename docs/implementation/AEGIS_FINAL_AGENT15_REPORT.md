# Aegis FINAL — Agent 15: Artifact Library & Core Packaging

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Baseline HEAD:** `2bd2513` (no commit, reset, clean, or deploy)
**Scope:** additive `.corepkg` package format for the existing `CoreInstaller`.

## Files

| File | Change |
|---|---|
| `crates/aegis-core-install/src/package.rs` | New: manifest model, strict JSON parser/serializer, roster identity validation, SHA-256 file verification, `validate_package` / `install_package` APIs |
| `crates/aegis-core-install/src/lib.rs` | Module wiring and public re-exports only |
| `crates/aegis-core-install/tests/core_package.rs` | New: 9 package-format and install-safety tests |
| `docs/implementation/AEGIS_FINAL_AGENT15_REPORT.md` | This report |

No CLI, other crate, workspace file, or dependency was changed. The legacy
`D:\SofiaDev\SofiaAdaLyra` repository was never touched. All tests use
`tempfile` roots.

## Package format

A package is either a directory containing `manifest.json` at its root, or raw
manifest JSON bytes for a self-contained package. The manifest carries:

- `schema` (`aegis.core.package.v1`) and `version` (major 1 accepted),
- `core_uuid` (the existing immutable Core identity field),
- `core_number`,
- `display_name` and optional `designation` expectations,
- `files` and `assets` arrays, each entry `{ "path", "sha256" }`.

No new identity fields are invented. Paths must be relative, forward-slash
separated, traversal-free, non-symlinked, and inside the package root. Digests
must be 64 hexadecimal characters and match the exact file bytes.

## Verification

- `cargo test -p aegis-core-install`: **34 passed, 0 failed, 0 ignored**
  - `core_installation`: 12
  - `core_package`: **9 new**
  - `core_regression`: 5
  - `enrollment_authorization`: 3
  - `lifecycle_integrity`: 5
- `cargo clippy -p aegis-core-install --all-targets -- -D warnings`: **exit 0, clean**
- `rustfmt --edition 2021 --check` on the three touched Rust files: **exit 0**

New tests cover: valid directory install + integrity verification; digest
mismatch rejection before any Core is created; prohibited Core #4; blank Core #7
requiring an owner name (and owner-supplied name acceptance); approved
name/designation mismatch; duplicate install refusal without overwrite;
`uninstall_preserve` non-destructiveness; byte-package install; path-traversal
rejection.

## Safety

- Validation happens **before** `CoreInstaller` is constructed or any Core
  storage is written, so a bad package produces no partial Core.
- Cores #4/#5 are always prohibited; #1/#2/#3/#6 must use their frozen roster
  display names; #7 requires a non-blank owner-supplied name.
- Installation delegates to `CoreInstaller::initialize`; it does not reimplement
  identity, storage, integrity sealing, or registration.
- Existing Cores are never overwritten: refusal is by Core UUID, by Core number,
  or by an existing unregistered database.
- Referenced files are checked for path escape, symlink/reparse redirection,
  regular-file status, and size limits before digest comparison.
- `uninstall_preserve` remains non-destructive; the test confirms the database
  and integrity manifest bytes are unchanged.

## Limitations

- Package assets are validated but not materialized into Core storage; the
  existing `CoreInstaller` has no asset-copy step, and asset placement remains
  owned by the embodiment asset library. Installation creates the Core identity
  and storage only.
- A byte package is manifest-only and may not reference external files.
- The approved-name/designation expectations mirror the frozen `aegis-roster`
  catalog locally because the package dependency set and `Cargo.toml` were
  frozen for this task; Core-number policy still comes from shared
  `aegis-contracts` lifecycle constants.
- Package content is integrity-checked, not authenticated at the publisher
  level. Trust in the package source must come from a separate authenticated
  channel, as with Core integrity manifests.
- If `CoreInstaller::initialize` fails after validation, its existing
  checkpoint/failure semantics are inherited unchanged; this task does not add
  a new cleanup policy that could destroy recoverable state.
