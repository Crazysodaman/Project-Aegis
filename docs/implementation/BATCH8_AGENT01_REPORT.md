# Batch 8 — Agent 01 Report (M1 Backup & Recovery)

**Status:** Implemented, tested, clippy-clean. No external deployment.

## Files
- `crates/aegis-backup/src/{backup.rs,manifest.rs,restore.rs,verify.rs,state.rs,lib.rs}`
- `crates/aegis-backup/src/{percore.rs,assets.rs}` (new)
- `crates/aegis-backup/tests/batch8_inventory.rs` (new)
- `docs/implementation/BATCH8_AGENT01_M1_BACKUP.md`

## Delivered
- Backup format **v2** (v1 still readable): per-file kind/sha256/bytes + per-Core
  database lists.
- Per-Core coverage: core.db, integrity manifest, memory.db, conversation.db,
  assets.db, social.db, embeddings.db, personal.db (optional), init marker, and
  asset files under `assets/`.
- Ownership validation (core UUID; core.db also database_instance_uuid), digest
  verification, atomic temp+rename, partial-restore refusal, missing-optional
  reporting, cross-Core contamination rejection.
- Live backup/restore verified on a disposable root (9 files incl. personal.db;
  continuity preserved after restore).

## Tests
`cargo test -p aegis-backup` → **18 passed** (12 existing + 6 new).
`cargo clippy -p aegis-backup --all-targets -- -D warnings` → clean.

## Limitations
Live-write consistency not handled (no SQLite backup API/WAL); asset cross-check
against assets.db not automatic; DPAPI owner.proof is machine-bound.

## Next
Optional SQLite backup API, asset-reference cross-check, whole-tree restore.
