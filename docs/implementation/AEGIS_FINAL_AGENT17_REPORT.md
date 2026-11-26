# Aegis FINAL — Agent 17: Backup, Restore & Disaster Recovery

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Baseline HEAD:** `2bd2513` (no commit, reset, clean, or deploy)
**Scope:** close the live-write consistency gap for backups with consistent
SQLite snapshots, including open databases with committed WAL data.

---

## Files

| File | Change |
|---|---|
| `crates/aegis-backup/src/snapshot.rs` | New: read-only `VACUUM INTO` snapshotting, SQL-parameter path quoting, sidecar detection |
| `crates/aegis-backup/src/backup.rs` | Snapshot every DB; live-source validation; re-seal Core integrity manifests from snapshots |
| `crates/aegis-backup/src/lib.rs` | Module wiring and live-consistency/limitation documentation |
| `crates/aegis-backup/tests/live_snapshot.rs` | New: 6 live-WAL and refusal tests |
| `docs/implementation/AEGIS_FINAL_AGENT17_REPORT.md` | This report |

No other crate, CLI, workspace file, or dependency was changed. All tests use
`tempfile`; no live `var/` data was read or written.

---

## Verification

- `cargo test -p aegis-backup`: **29 passed, 0 failed, 0 ignored**
  - `backup_restore`: 12
  - `batch8_inventory`: 6
  - `batch8_qa`: 2
  - `live_snapshot`: **6 new**
  - `ownership_contamination`: 3
- `cargo clippy -p aegis-backup --all-targets -- -D warnings`: **exit 0, clean**
- New tests: committed WAL sidecar restored while writer open; committed WAL
  `core.db` snapshotted and re-sealed; corrupted archive refused; incompatible
  manifest version refused; missing optional DBs non-fatal; non-empty target
  refused and preserved.

---

## Consistency method

- Every database (Platform, Safe, Core, and each sidecar) is snapshotted through
  SQLite with `VACUUM INTO` over a read-only connection, so committed WAL content
  is captured without ever copying `-wal`.
- The snapshot path is passed as a bound SQL parameter, so quotes and other SQL
  metacharacters in destination paths are safe.
- Raw file copy is only a fallback when `VACUUM INTO` fails **and** no
  `-wal`/`-shm`/`-journal` sidecar exists (a genuinely closed/self-contained
  database). A live sidecar with no successful snapshot is refused.
- Core integrity manifests are re-sealed for the snapshot, preserving Core UUID,
  identity digest, database instance UUID, initialization mode, schema revision,
  and Core number. Identity, ownership, and instance UUID are re-validated on the
  copied snapshot; ownership checks were not weakened.
- The v2 backup manifest, per-file SHA-256/byte digests, allowlist validation,
  and write-manifest-last atomic semantics are unchanged.

## Limits

- A Core database with uncheckpointed WAL has no sealed digest for that committed
  state. The live database is validated with SQLite `integrity_check`, manifest
  identity, and ownership, then the snapshot is re-sealed; the backup itself is
  still digest-verified.
- Read-only WAL access requires the `-shm` sidecar to be accessible to the
  backup process; otherwise backup fails closed rather than copying `-wal`.
- Platform identity and the Core registry are read from the source before the
  Platform snapshot; a registry mutation in that narrow window can make the
  backup fail verification rather than silently drop a Core.
- Integrity remains content integrity, not authenticity; a party able to rewrite
  both a database and its manifest can create a self-consistent pair.
