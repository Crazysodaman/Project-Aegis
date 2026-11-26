# Batch 6 — Recovery Manifest

**Date:** 2026-11-26
**HEAD:** `2bd2513` (no commit)
**Purpose:** Preserve all Batch 3–6 uncommitted work without committing or
overwriting Batch 4 material.

## Snapshots

| Snapshot | Location | Contents |
|---|---|---|
| Batch 4 (preserved) | `.tmp/b4-recovery/` | earlier patch + untracked copies |
| Baseline commit | git `2bd2513` | 182-file pre-Batch-3 tree |
| Batch 6 | `.tmp/b6-recovery/` | current diff patch + all untracked files |

`.tmp/b6-recovery/` contains:
- `tracked-changes.patch` — `git diff 2bd2513` for all tracked modifications
  (excludes `.tmp/`), binary-safe.
- `untracked/` — every untracked file at its repository-relative path.
- `untracked-manifest.txt` — list of copied untracked files.

## Working-tree state at snapshot time

- Modified tracked: 29.
- Untracked: 73 (45 source/test files, 28 docs).
- Ignored top-level: `target/`, `var/`, `.tmp/`.
- `git ls-files var` = 0 (no live DB/owner proof tracked).
- No secrets found in tracked or untracked source/config/docs.

## Recovery procedure (no data loss)

1. Do not reset or clean. The working tree is authoritative.
2. To reconstruct against the baseline elsewhere:
   `git checkout 2bd2513 -- .` in a *copy*, then
   `git apply .tmp/b6-recovery/tracked-changes.patch`, then copy
   `.tmp/b6-recovery/untracked/*` into place.
3. Batch 4 material is retained separately under `.tmp/b4-recovery/`.

## Integrity of live data

Someone must NOT copy `var/` or `.tmp/` into Git. Live Core databases and
`owner.proof` remain ignored. This snapshot is local only.
