# Batch 7 — Recovery Manifest

**Date:** 2026-11-26
**HEAD:** `2bd2513` (no commit)

## Snapshots

| Snapshot | Location | Preserved |
|---|---|---|
| Baseline commit | git `2bd2513` | 182 files |
| Batch 4 | `.tmp/b4-recovery/` | yes (not overwritten) |
| Batch 6 | `.tmp/b6-recovery/` | yes (not overwritten) |
| Batch 7 | `.tmp/b7-recovery/` | current |

`.tmp/b7-recovery/`:
- `tracked-changes.patch` — `git diff 2bd2513` (tracked modifications, `.tmp` excluded)
- `untracked/` — every untracked file at repo-relative path
- `untracked-manifest.txt`
- `SNAPSHOT_HASHES.txt` — SHA-256 for B4/B6/B7 patches

## Recorded SHA-256 (before Batch 7 work)

| Artifact | SHA-256 |
|---|---|
| `.tmp/b4-recovery/tracked-changes.patch` | `BE7C483E13DBB1ACE15D4643E7EBE3B50695AB802FF14760E4CA61DBF9974618` |
| `.tmp/b6-recovery/tracked-changes.patch` | `5374B786F91C48422D84C36AD9C3E2A4382E147B13EBD7E7032865F09B3B6386` |
| `.tmp/b7-recovery/tracked-changes.patch` | matches B6 at snapshot time (`5374B786…`) |

The Batch 7 snapshot is refreshed at end of batch to include Batch 7 changes and
new hashes.

## Recovery procedure (no data loss)

1. Do not reset/clean; the working tree is authoritative.
2. In a copy of the baseline: `git checkout 2bd2513 -- .`, then
   `git apply .tmp/b7-recovery/tracked-changes.patch`, then copy
   `.tmp/b7-recovery/untracked/*` into place.
3. B4 and B6 snapshots remain available independently.

## Live data

`var/` (databases, owner.proof) and `.tmp/` are git-ignored. Do not add them to
Git. Recovery snapshots are local only.
