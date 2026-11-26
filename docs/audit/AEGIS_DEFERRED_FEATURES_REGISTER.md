# K. Deferred Features Register

Features approved or designed but not scheduled/implemented, with the reason and
the decision source. "Deferred" does not mean cancelled.

| Requirement | Status | Reason / source |
|---|---|---|
| Adult romance/sexuality/intimacy/consent (A-04) | DEFERRED / PROVISIONAL | `BLOCKERS.md`, `DECISIONS.md`; no implementation; eligibility undecided |
| Encryption at rest | DEFERRED | separate SQLite + OS permissions chosen (`BATCH1`, `CORE_STORAGE_AND_IDENTITY`) |
| B-01 Hephaestus backup placement | DEFERRED | owner decision pending (legacy `06_OWNER_DECISIONS_LOCKED`) |
| B-06 retention/RPO/RTO | DEFERRED | owner decision pending |
| Production cutover / legacy Sofía DB retirement | DEFERRED | owner selected testing activation only |
| Artemis Fleet agent + cross-host inference | BLOCKED (owner auth) | `AEGIS_FINAL_OWNER_ACTIONS.md` |
| Debian 13 runtime/service/reboot | BLOCKED (no VM) | deployment reports |
| Native GUI / mobile | DEFERRED (deps/approval) | `AEGIS_FINAL_KNOWN_LIMITATIONS.md` |
| Microphone STT, playback, streaming, voice selection | DEFERRED (hardware/models) | voice audits |
| GPU avatar renderer / rig / decoded art | DEFERRED (deps) | embodiment audit |
| OS module sandbox, signatures, upgrade/rollback | DEFERRED | module audit |
| Cross-host Artifact Library replication | DEFERRED (owner auth/design) | artifact reports |
| 55-module catalog implementation | DEFERRED (scope) | catalog register |
| Social agency, invitations, quiet periods, budgets | DEFERRED (design) | social audit |
| Voluntary governance/collective decisions | LEGACY DESIGN ONLY | legacy governance docs; absent from v0.2 blueprint |
| Dual cognition/model routing | DEFERRED | blueprint §7; not wired |
| Calendar provider | DEFERRED | world audit |
| Developer workspace | DEFERRED | UI audit |

## Superseded / corrected

| Earlier statement | Superseding decision |
|---|---|
| v0.1 blueprint authoritative | v0.2.0 (AEGIS-MAB-002); external v0.3.0 referenced |
| Python desktop primary UI | Rust TUI-first |
| Historical DB import | `fresh_no_migration`; legacy DB read-only preserved |
| "Gaming-load yield not live-verified" | Real game-process yield verified (Batch 4); gaming-safe/FPS still unclaimed |
| "Social/voice/embodiment/fleet absent" | Implemented/partial in later batches |
| "No live inference" | Live Ollama chat/embeddings verified 2026-11-25 |
| Batch 8 M7 "no defects" | D20-02/03 and others found by later agents |
| "Legacy repo untouched" | Aegis never wrote it, but audit observed it dirty at `d1666be`; claim not provable from repo state |
| Test-count 350 in README | 942 actual (docs need update) |

## Unresolved owner questions

1. Which spec governs: mirrored v0.2.0 or external v0.3.0?
2. Was #6 Vera implementation/installation explicitly authorized?
3. Wardrobe canonical count: 275/23/47 vs later 295/51?
4. Adult eligibility/consent model (A-04) and whether to implement at all?
5. Backup placement/authority (B-01) and retention (B-06)?
6. Should legacy voluntary-governance/collective decisions be retained?
7. How to handle the already-dirty legacy repository and uncommitted Aegis tree?
