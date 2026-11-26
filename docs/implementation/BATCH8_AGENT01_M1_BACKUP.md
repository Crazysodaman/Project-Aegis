# Batch 8 — Agent M1: Backup & Recovery (Milestone 1)

Status: **IMPLEMENTED / TESTED**
Scope: `crates/aegis-backup/src/**`, `crates/aegis-backup/tests/**`, this document.
No workspace manifest, lockfile, CLI, other crate, or completion-ledger changes were made.

## 1. Persistent-state inventory

### Platform root (always included)

| Path | Kind | Required |
| --- | --- | --- |
| `platform.db` | Platform SQLite database | yes |
| `safe.db` | Safe SQLite database | yes |
| `owner.proof` | Owner proof (bytes copied verbatim, never parsed/logged) | yes |

### Per-Core (under `cores/<uuid>/`)

| File | Role | Ownership table | Required |
| --- | --- | --- | --- |
| `core.db` | Core identity + storage | `core_identity` (`core_uuid`, `database_instance_uuid`) | **yes** |
| `core.integrity.json` | Integrity manifest | (validated by `verify_core`) | yes |
| `memory.db` | Memory store | `core_memory_ownership` (`core_uuid`) | optional |
| `conversation.db` | Conversations | `conversation_ownership` (`core_uuid`) | optional |
| `assets.db` | Asset library metadata | `asset_ownership` (`core_uuid`) | optional |
| `social.db` | Relationships | `relationship_ownership` (`core_uuid`) | optional |
| `embeddings.db` | Embeddings | `embedding_ownership` (`core_uuid`) | optional |
| `personal.db` | Personal data (future) | `personal_ownership` (`core_uuid`) | optional |
| `assets/<asset_id>/<safe-name>` | Asset blobs | referenced by `assets.db` | optional |

Initialization markers: `init/<uuid>.initialization.marker` (copied when present).

`run/`, heartbeats, stop sentinels, and SQLite `-wal`/`-shm` sidecars are **not**
part of a backup (see limitations).

## 2. Format v2

New backups write:

* `schema = "aegis.backup.manifest.v2"`
* `format_version = 2`
* every `BackupFile` carries `kind`, `sha256`, and `bytes`
* every `BackupCore` carries:
  * `databases: [BackupDatabase]` — `file_name`, `relative_path`, `kind`,
    `ownership_table`, `sha256`, `bytes` (always contains `core.db`)
  * `missing_optional_databases: [String]` — optional DBs absent at backup time
* asset blobs appear as `CoreAssetFile` entries with their relative
  `cores/<uuid>/assets/...` path preserved.

Public constants: `BACKUP_FORMAT_VERSION`, `BACKUP_MANIFEST_SCHEMA`,
`CORE_DATABASE_FILE`, plus the v1 constants.

### v1 compatibility / migration

* v1 manifests (`aegis.backup.manifest.v1`, version `1`) remain readable,
  verifiable, and restorable.
* The new `BackupCore` fields use `#[serde(default)]`, so a v1 manifest without
  a per-Core database list deserializes with empty lists.
* Validation dispatches on the `(schema, format_version)` pair; v2-only checks
  (database list completeness, sidecar ownership declaration) are skipped for
  v1.
* Restore of a v1 backup still verifies `core.db`, its integrity manifest, and
  the platform/safe markers. It simply has no sidecars to restore.
* Migration path: take a new v2 backup; v1 backups need no rewrite.

## 3. Ownership and integrity checks

Backup (`backup()`):

* `core.db`: identity row read, `core_uuid` matched, integrity manifest and
  SQLite integrity verified, `database_instance_uuid` compared against both the
  copied identity and the integrity manifest.
* Every present sidecar: opened read-only, ownership table queried, exactly one
  row required, `core_uuid` must match. A present DB with a missing/foreign
  ownership row is fatal.
* A missing `core.db` is fatal. Missing optional DBs are recorded in
  `missing_databases` (report) and `missing_optional_databases` (manifest), never
  fatal.
* Every copied byte stream is hashed source-side and destination-side; mismatch
  aborts and the manifest is not written.

Restore (`restore()`):

* The whole backup is verified (`verify_backup`) before any target write.
* Every manifest file is re-hashed into staging/replacement.
* `core.db` identity/integrity and every sidecar ownership row are re-checked in
  the restored tree.
* Fresh restores stage into a sibling directory, verify, then atomically rename
  into place. A failure leaves no partial target.
* Existing-target restores first preserve the target, then replace each file via
  temp + atomic rename. The pre-existing content remains in the rollback backup.

Every per-file copy (backup and restore) uses a temporary sibling plus atomic
rename; the manifest itself is written last and atomically.

## 4. What is and is not included

Included: platform/safe DBs, owner proof, all per-Core DBs listed above,
`core.integrity.json`, init markers, and every regular file under
`<core>/assets/` (symlinks/reparse points rejected, paths contained, bounded by
file count/depth/total bytes).

Not included: `run/` scratch, heartbeats, stop sentinels, SQLite `-wal`/`-shm`,
logs, module sandboxes, and any path outside the allowlist. Backups are content
integrity, not signatures; a party able to rewrite both data and manifest can
forge a self-consistent pair.

## 5. Recovery procedure

1. Stop all Cores and the Platform (backup refuses on fresh heartbeats or live
   `core_runtime_state`).
2. `backup <root> <new-empty-destination>` (destination must not exist and must
   not overlap the source).
3. Confirm with `verify_backup <destination>` (status `Verified`, no issues).
4. Restore into a missing/empty target with `restore <backup> <target>`.
   To overwrite an existing root, opt into `allow_existing` so the current root
   is first preserved as a rollback backup.
5. Re-run `verify_backup`/Core `verify` on the restored root.

## 6. Tests and results

Command: `cargo test -p aegis-backup`

* `tests/backup_restore.rs` — 12 tests (pre-existing) — all pass.
* `tests/batch8_inventory.rs` — 6 new tests — all pass:
  * `full_inventory_backup_and_restore_is_isolated` — platform + 2 Cores +
    fabricated sidecars (ownership rows) + nested assets + optional `personal.db`
    on one Core only; backup, verify, restore into a fresh root, then assert
    identity, ownership, digests, marker isolation, and asset bytes.
  * `cross_core_database_ownership_is_rejected` — swapping Core B's `memory.db`
    into Core A fails the backup and writes no manifest.
  * `missing_optional_databases_are_reported_not_fatal` — installer-only Cores
    (no sidecars) still back up/restore; all 6 optional DBs reported missing.
  * `legacy_v1_manifest_without_database_list_is_readable_and_restorable`.
  * `corrupted_partial_backup_is_refused_without_writing_target`.
  * `interrupted_partial_target_is_refused_and_preserved`.

Total: **18 passed, 0 failed**.

Command: `cargo clippy -p aegis-backup --all-targets -- -D warnings` — **clean**.

## 7. Limitations

* A backup must not run while another process writes the databases; raw byte
  copies of a live SQLite DB are not transactionally consistent. WAL sidecars
  are intentionally excluded.
* Asset blobs are discovered by a bounded safe walk of `<core>/assets/`. Files
  referenced by `assets.db` but stored elsewhere are outside this allowlist.
* Empty asset directories are not recreated on restore (no files to copy).
* Existing-target restore is per-file atomic with a preserved rollback copy; it
  is not a single atomic swap of the whole tree.
* `personal.db` support assumes a `personal_ownership` table with a `core_uuid`
  column; a present `personal.db` without it is rejected (fail-closed).
* `owner.proof` restored under a different Windows user/machine will not decrypt
  (DPAPI); bytes are preserved exactly.
* Content integrity is not authenticity.
