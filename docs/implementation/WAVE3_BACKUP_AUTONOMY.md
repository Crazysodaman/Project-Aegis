# Wave 3b — Backup coverage for per-Core `autonomy.db`

Status: **IMPLEMENTED / TESTED**
Scope: `crates/aegis-backup/src/manifest.rs`, `crates/aegis-backup/src/percore.rs`,
`crates/aegis-backup/tests/**`, and this document. No workspace manifest,
lockfile, CLI, other crate, or completion-ledger changes were made.

## 1. Coverage change

`autonomy.db` is now a recognized optional per-Core database, handled exactly
like the other lazily created sidecars (for example `consent.db` and
`possessions.db`).

| File | Kind | Ownership table | Required |
| --- | --- | --- | --- |
| `autonomy.db` | `BackupFileKind::CoreAutonomyDatabase` | `autonomy_ownership` (`core_uuid`) | optional |

The file is registered once in `SIDECAR_DATABASES` (`percore.rs`) and
`CoreAutonomyDatabase` is part of `is_core_database_kind` (`manifest.rs`), so the
existing generic machinery covers it end to end:

* Backup discovers `cores/<uuid>/autonomy.db`, opens it read-only, and proves the
  single `autonomy_ownership` row names the owning Core before snapshotting.
* The snapshot is recorded as a `BackupFile` (`kind`, `sha256`, `bytes`) and a
  `BackupDatabase` (`file_name`, `relative_path`, `kind`, `ownership_table`,
  `sha256`, `bytes`).
* Restore re-verifies the manifest digest and the restored ownership row.
* When absent, `autonomy.db` is reported in `missing_optional_databases`
  (manifest) and `missing_databases` (report) and is never fatal.

No bespoke autonomy code path was added; the allowlist and ownership checks
remain the single source of truth.

## 2. Format compatibility

* New backups stay on schema `aegis.backup.manifest.v2` / format version `2`.
* v1 manifests remain readable, verifiable, and restorable. The new kind only
  appears in v2 database lists; v1 validation still skips v2-only database
  checks.
* All ownership checks are unchanged and still fail closed: a present
  `autonomy.db` whose `autonomy_ownership` table is missing, has not exactly one
  row, or names another Core aborts the backup with `IntegrityFailed` and writes
  no manifest.

## 3. Tests and results

Command: `cargo test -p aegis-backup` — **34 passed, 0 failed**.

`tests/batch8_inventory.rs` — 8 tests, including the new
`autonomy_database_is_backed_up_and_restored_with_ownership_and_digest`:

* present `autonomy.db` on one Core is recorded with kind
  `CoreAutonomyDatabase`, ownership table `autonomy_ownership`, a 64-char
  digest, and matching file/database byte counts;
* the other Core reports it as an optional missing database in both
  `missing_optional_databases` and `missing_databases`;
* backup verifies, restore succeeds, the restored ownership row names the owning
  Core, the marker content survives, and the restored file digest equals the
  manifest digest;
* the Core without `autonomy.db` does not gain one on restore.

Counts updated for the new sidecar:

* `STANDARD_SIDECARS` now includes `("autonomy.db", "autonomy_ownership")`
  (8 entries; `personal.db` is still exercised separately).
* `full_inventory_backup_and_restore_is_isolated`: Core A databases `9 -> 10`,
  Core B databases `8 -> 9`.
* `missing_optional_databases_are_reported_not_fatal`:
  `missing_optional_databases` `8 -> 9` per Core.

Command: `cargo clippy -p aegis-backup --all-targets -- -D warnings` — **clean**.
