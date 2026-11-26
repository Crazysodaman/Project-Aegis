# AEGIS FINAL — Release Manifest

**Date:** 2026-11-26
**HEAD:** `2bd2513`
**Committed:** No (owner approval required)
**Release state:** NOT RELEASED · PRODUCTION NO-GO

## Working tree

- Modified tracked: **52**
- Untracked: **200** (source/tests/docs; includes 20 agent reports)
- Ignored: `target/`, `var/`, `.tmp/`, DB/proof files
- Legacy `D:\SofiaDev\SofiaAdaLyra`: untouched

## Dependencies

- New third-party crate: **`chrono-tz = 0.10.0`** (Batch 8; IANA/DST), workspace +
  `aegis-platform`. No new crate added in AEGIS-FINAL.
- System dependency: `curl` (live weather; Windows `curl.exe`).

## Changed areas (FINAL programme)

Platform lifecycle/tests; Fleet eligibility; GPU queue/placement; World refresh/
distribution; Personal expressed emotion/relationship events; Memory consolidation
/session summary; Event→emotion pipeline; SAPI voice style; Avatar state/sequence;
Wardrobe constraints; GUI world/personal; CLI package/voice + TUI snapshots;
Social events/awareness; Module dependency/version; `.corepkg`; Security tests;
Live-safe backup; Installer preflight; Reliability tests; QA.

## Evidence

- Logs: `.tmp/final-{fmt-check,clippy,build,test-workspace,live-artifact}.log`;
  agent logs in `.tmp/` from each milestone.
- Agent reports: `docs/implementation/AEGIS_FINAL_AGENT01..20_REPORT.md`.
- Programme reports: `AEGIS_FINAL_*.md`.

## Recovery

| Snapshot | SHA-256 (patch) |
|---|---|
| B4 | `BE7C483E…9974618` |
| B6 | `5374B786…F09B3B6386` |
| B7 | `699567F0…A8B48F69C2` |
| B8 | `A609F33A…5D95BA8000F` |
| FINAL | see `.tmp/final-recovery/SNAPSHOT_HASHES.txt` (refreshed at end) |

## Classification

- Development: **GO**
- Deployment: **NO-GO**
- Production: **NO-GO**
- Independent external acceptance: **NOT SATISFIED**

## Release checklist

- [x] Source builds; fmt/clippy/tests clean
- [x] Recovery snapshots preserved + FINAL snapshot
- [x] Agent reports + programme reports
- [ ] Owner commit approval
- [ ] Artemis deployment/verification
- [ ] Debian runtime verification
- [ ] Native GUI decision
- [ ] D20-02/D20-03 disposition
- [ ] Independent external QA
- [ ] Owner acceptance
- [ ] Production deployment authorization
