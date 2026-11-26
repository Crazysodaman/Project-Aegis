# F. Final Programme Reconciliation

**Report audited:** `docs/implementation/AEGIS_FINAL_*.md`
**Verified against:** working tree at `2bd2513` + logs.

| Claim | Verdict | Evidence / correction |
|---|---|---|
| HEAD `2bd2513`, branch `master`, no commit | VERIFIED | `git rev-parse HEAD`; `git log -1`; nothing staged |
| 52 modified tracked files | VERIFIED | `git diff --name-only` = 52 |
| 200 untracked files | **FALSE (stale)** | actual `git ls-files --others` = **212**; `git status --porcelain` 259 lines |
| 942 passed / 0 failed / 1 ignored | VERIFIED for `final-test-workspace.log` | 160 result lines aggregate to 942/0/1 |
| Ignored test was run and passed | VERIFIED separately | `.tmp/final-live-artifact.log`: 131,072 B, SHA verified |
| fmt exit 0 | VERIFIED independently | but `final-fmt-check.log` cited by QA **does not exist** |
| clippy/build exit 0 | VERIFIED by logs | timestamps after source edits |
| 20/20 agent reports | VERIFIED (existence) | Agent 20 is internal, not external |
| "Zero new dependencies" in FINAL | TRUE for FINAL window | working tree vs HEAD adds `chrono-tz` (+14 transitive crates) from Batch 8; `Cargo.lock` +129 lines |
| Live SAPI TTS/STT 184,334 B + transcript | PARTIALLY VERIFIED | Batch 7 log proves 84,904 B TTS and 148,836 B TTS→STT transcript; `184334` appears only in prose, and the 942 run skipped SAPI (0.00s) |
| Live avatar 18 frames / 1,106,892 B | **UNVERIFIED** | Batch 7 log shows 12 frames / 737,928 B; 18-frame figure only in prose (arithmetically consistent with 12 fps × 1500 ms) and final live test was skipped |
| Live weather | VERIFIED (Batch 8) | `.tmp/b8-live-weather.log` Open-Meteo observation; final run skipped (0.00s) |
| Live-safe backup/restore | VERIFIED (Batch 8 + tests) | `b8-live-backup.json`, `b8-live-restore.json`, `live_snapshot` tests |
| CLI install/world/chat/personal backup smoke | VERIFIED (Batch 8 logs) | new FINAL package/voice smoke is prose-only (`115110` in no log) |
| "Live tests are part of 942" | **MISLEADING** | env-gated live tests early-return; 942 includes no-op "passes" |
| Development GO / Deployment/Production NO-GO | CONSISTENT | matches uncommitted, undeployed, internal-QA-only state |

## Corrections to carry into the ledger

1. Working tree untracked count is 212, not 200.
2. FINAL live evidence partly reuses Batch 7/8 logs; several FINAL-specific live
   numbers are prose-only and must not be cited as independently captured.
3. Env-gated live tests are skipped in the 942 total; the suite is not live proof.
4. The dependency claim must be stated relative to the commit (chrono-tz added in
   Batch 8), not "no dependencies."
5. `final-fmt-check.log` is missing; formatting is clean but the evidence citation
   is inaccurate.
6. Batch 8 "M7 no defects" was corrected by Agent 16/20 (D20-02/03 etc.).

## Continuing defect status (verified)

| Defect | Status | Severity |
|---|---|---|
| D20-02 verify-backup marker swap | STILL PRESENT | Medium |
| D20-03 safe.db deletion recreates inactive STOP | STILL PRESENT | Medium |
| Event claim/effects non-atomic | STILL PRESENT (accepted) | Low |
| Oversized-retrieval orphan user message | STILL PRESENT | Low |
| Duplicate/stampeding world refresh | STILL PRESENT | Low |
| Documentation drift (350 vs 942; contradictory feature lists) | PARTIALLY MITIGATED | Medium |
| Uncommitted tree | OPEN | High (process) |
