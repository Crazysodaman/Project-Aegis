# I. Release Defect Register

**Repo:** `D:\Aegis` @ `2bd2513` (dirty). Read-only audit. **No fixes applied.**

| ID | Severity | Defect | Evidence | Recommended remediation | Status |
|---|---|---|---|---|---|
| D20-01 | High (process) | Entire reviewed tree uncommitted; HEAD has only 354-test baseline | `git rev-parse HEAD`; 52 modified + 212 untracked | Owner-authorized commit checkpoint; signed/reproducible release | OPEN |
| D20-02 | Medium | `verify-backup` can report `Verified` after swapping `platform.db`/`safe.db` and rewriting digests; restore fails closed | `crates/aegis-backup/src/verify.rs` (no marker check) vs `restore.rs`; test `ownership_contamination.rs` | Make `verify_backup` validate Platform/Safe application/schema markers | OPEN |
| D20-03 | Medium | Deleting `safe.db` then any Platform-starting CLI recreates inactive STOP + empty audit (one-step reset) | `aegis-safe/src/store.rs` initialize; test `stop_and_owner_adversarial.rs` | Anchor STOP/audit externally or require explicit owner-authorized Safe recovery | OPEN |
| D20-04 | Medium | Docs contradict/stale: README 350 tests, social/voice listed not implemented, live-inference contradictions; ledger rows conflict | `README.md`, `CURRENT_STATE.md`, `TEST_EVIDENCE.md`, `MASTER_COMPLETION_LEDGER.md` | Regenerate from one evidence ledger; delete stale duplicate rows | PARTIALLY MITIGATED |
| D20-05 | Low | Concurrent stale world refresh stampedes provider calls; unavailable weather re-polls every trigger; `world_events` grows | `aegis-platform/src/world.rs`; `integration_reliability.rs` | Single-flight/coalesced refresh + backoff + retention bound | OPEN |
| D20-06 | Low | Oversized retrieval token persists user message then rejects, leaving orphan row | `aegis-conversation/src/engine.rs`; `integration_reliability.rs` | Build request before persisting, or roll back on failure | OPEN |
| D20-07 | Low | Event claim and effects are separate transactions; crash can claim without all effects (no duplicates) | `aegis-core-sdk/src/events.rs`, `personal.rs` | Transactional outbox / atomic claim+effects | OPEN (accepted) |
| D20-08 | Low | FINAL live figures (184,334 B TTS; 18-frame avatar; package/voice smoke 115,110 B) exist only in prose; env-gated live tests skipped in 942 | FINAL docs vs `.tmp/b7-*`, `.tmp/b8-*` logs | Re-run and log live evidence; label reused evidence | OPEN |
| D20-09 | Medium | Sofía UUID/Constitution not enforced; arbitrary UUID installs for #1 | `aegis-cli` core install; roster has no UUID field; test roots | Bind approved number→UUID+Constitution; reject mismatches; negative tests | OPEN |
| D20-10 | High | Approved capability manifest over-advertises 23 capabilities to every Core; SDK context wires a subset | `aegis-core-sdk/src/capability.rs`, `lib.rs` | Per-Core evidence-backed manifest; report unavailable | OPEN |
| D20-11 | High | Roster duplicated in 5 places (roster/profiles/package/lifecycle/SQL); drift already present (#6 status, integrity schema) | see `AEGIS_REPOSITORY_ARCHITECTURE_MAP.md` | Single canonical catalog + drift tests | OPEN |
| D20-12 | Medium | `integrity-manifest.schema.json` `additionalProperties:false` omits real fields | `contracts/core/v1/integrity-manifest.schema.json` vs `aegis-contracts/src/integrity.rs` | Version/update schema + validation tests | OPEN |
| D20-13 | Medium | Module example declares `wasm` but runtime rejects `wasm`; example not runnable | `aegis-modules/examples/hello.manifest.json`, `runtime.rs` | Fix example or support wasm; add runtime test | OPEN |
| D20-14 | Medium | Module packages excluded from backup and uninstall deletion | `aegis-backup`, `aegis-installer` | Include `modules/` in backup/uninstall policy | OPEN |
| D20-15 | Medium | UI `run/status` panels have no emitter; normal state is UNAVAILABLE/STALE | `aegis-ui/src/model.rs`; no producer | Add atomic emitter or connect live data source | OPEN |
| D20-16 | Medium | GUI Core workspace reuses one root `personal.json`/outfit for all Cores without `core_uuid` check | `aegis-ui/src/gui.rs` | Scope snapshot per Core; reject mismatch | OPEN |
| D20-17 | High | GUI session token can drive owner-authorized lifecycle/STOP actions when mutations enabled | `aegis-ui/src/actions.rs` | Require owner proof/scoped grant; adversarial tests | OPEN |
| D20-18 | Medium | Sidecar ownership checks (backup/verify) check only `core_uuid`, not `database_instance_uuid`; some stores adopt DBs with missing ownership rows | `aegis-backup/src/percore.rs`; conversation/assets/social/embeddings stores | Require instance UUID; reject missing ownership | OPEN |
| D20-19 | Low | `safe_audit` append-only enforced by API only, no UPDATE/DELETE triggers | `aegis-safe/src/store.rs` | Add DB triggers | OPEN |
| D20-20 | Medium | World freshness uses snapshot `generated_at`, not provider `observed_at`; stale observations can sit in a fresh snapshot | `aegis-platform/src/world.rs` | Base freshness on observation time | OPEN |
| D20-21 | High | Relationship model is a `trust:u8` scalar + free text, contradicting blueprint structured/asymmetric states | `aegis-social/src/relationships.rs` | Structured relationship dimensions/history | OPEN (design) |
| D20-22 | High | No adult/consent/eligibility implementation; "fail-closed" is absence | no code; `BLOCKERS.md` A-04 | Implement consent registry/action gating or formally defer | OPEN (deferred) |
| D20-23 | High | 55-module catalog absent; 0/55 implemented | see module register | Recover catalog + implement packages | OPEN |
| D20-24 | High | Artemis not deployed; Debian not verified; no OS service install | deployment reports | Owner-authorized commissioning | BLOCKED |

Severity reflects impact on the original vision/production; Medium/Low items are
not release-blocking for local development but are for production/acceptance.
