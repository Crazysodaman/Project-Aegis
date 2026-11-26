# Batch 8 — Recovery Report

**Date:** 2026-11-26
**HEAD:** `2bd2513` (no commit)

## Snapshots preserved

| Snapshot | Location | Status |
|---|---|---|
| Baseline commit | git `2bd2513` | 182 files |
| Batch 4 | `.tmp/b4-recovery/` | preserved |
| Batch 6 | `.tmp/b6-recovery/` | preserved |
| Batch 7 | `.tmp/b7-recovery/` | preserved |
| Batch 8 (pre-work) | `.tmp/b8-recovery/` | patch + untracked + hashes |

`.tmp/b8-recovery/` contains `tracked-changes.patch`, `untracked/`,
`untracked-manifest.txt`, `SNAPSHOT_HASHES.txt` (B4/B6/B7/B8). The Batch 8
snapshot is refreshed after implementation to include all Batch 8 changes.

## Application-level backup (M1, format v2)

- Now backs up per-Core `core.db`, `core.integrity.json`, `memory.db`,
  `conversation.db`, `assets.db`, `social.db`, `embeddings.db`, `personal.db`
  (optional), init marker, and asset files under `assets/`, plus
  `platform.db`, `safe.db`, `owner.proof`.
- Ownership validation per DB (core UUID; core.db also instance UUID); SHA-256
  verification on backup and restore; atomic temp+rename; partial restore refused.
- **v1 manifests remain readable**; v2 adds per-file kinds/digests and per-Core
  database lists. Missing optional DBs are reported, not fatal.
- WAL `-wal`/`-shm` and live-write consistency are not handled (see limitations);
  quiesce Cores before backup for strict consistency.

## Live backup/restore evidence (disposable root)

- Backup: 9 files, including `core.db`, `memory.db`, `conversation.db`,
  `personal.db`; manifest lists ownership tables and digests.
- Restore into a fresh root: continuity `last_active_at`/`last_session_id`
  preserved; `core verify` = `verified`; diagnostics shows the Core.

## Recovery procedure

1. Do not reset/clean; the working tree is authoritative.
2. Reconstruct pre-Batch-8 state: apply `.tmp/b8-recovery/` or the current
   `.tmp/b7-recovery/` against `2bd2513`.
3. Application data: use `aegis restore --from <backup-dir> --root <target>`;
   verify with `aegis diagnostics` and `aegis core verify`.
4. Keep `var/`, `owner.proof`, and all recovery bundles out of Git.

## Limitations

- Backup is not live-write consistent; SQLite backup API/VACUUM INTO not used.
- Asset files are copied from the Core asset tree; cross-check against
  `assets.db` references is not automatic.
- `owner.proof` is DPAPI-bound (Windows); restore on another machine may not
  decrypt it.
