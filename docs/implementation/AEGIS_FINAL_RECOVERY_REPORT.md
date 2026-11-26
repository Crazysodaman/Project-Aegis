# AEGIS FINAL — Recovery Report

**HEAD:** `2bd2513` · no commit

## Recovery snapshots (all preserved)

| Snapshot | Location | SHA-256 (patch) |
|---|---|---|
| Batch 4 | `.tmp/b4-recovery/` | `BE7C483E…9974618` |
| Batch 6 | `.tmp/b6-recovery/` | `5374B786…F09B3B6386` |
| Batch 7 | `.tmp/b7-recovery/` | `699567F0…A8B48F69C2` |
| Batch 8 | `.tmp/b8-recovery/` | `A609F33A…5D95BA8000F` |
| FINAL | `.tmp/final-recovery/` | refreshed at end of programme (hash in `SNAPSHOT_HASHES.txt`) |

Each contains `tracked-changes.patch`, `untracked/`, `untracked-manifest.txt`,
and hashes. Do not reset/clean; the working tree is authoritative.

## Application backup (FINAL)

- Format v2; covers `platform.db`, `safe.db`, `owner.proof`, and per-Core
  `core.db`, integrity manifest, `memory.db`, `conversation.db`, `assets.db`,
  `social.db`, `embeddings.db`, `personal.db` (optional), init marker, assets.
- **Live-write-safe:** every DB snapshotted via read-only `VACUUM INTO` (committed
  WAL included without copying `-wal`); raw copy only when no WAL/journal exists;
  otherwise the backup fails closed.
- Ownership + SHA-256 verification; manifests re-sealed from the snapshot; v2
  atomic write-manifest-last; v1 manifests remain readable.
- Live backup/restore verified on disposable roots; restore preserved personal
  continuity/session and core integrity `verified`.

## Recovery procedure

1. Do not reset/clean. Reconstruct against `2bd2513` using the FINAL (or Batch 8)
   patch + untracked copies if needed.
2. For data: `aegis restore --from <backup-dir> --root <target>`, then
   `aegis diagnostics` and `aegis core verify`.
3. Keep `var/`, `owner.proof`, and recovery bundles out of Git.

## Remaining recovery limitations

- `owner.proof` is DPAPI-bound (Windows/current user); cross-machine restore may
  not decrypt it.
- Concurrent live writes are handled per-DB via `VACUUM INTO`, but the set of DBs
  is not a single atomic snapshot across all Cores.
- Content integrity is SHA-256, not publisher authenticity.
