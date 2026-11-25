# Stage 1-3 / Stage 5 - Independent Pre-Integration QA Review

**Review status:** **NOT APPROVED - PRE-INTEGRATION, VOLATILE SOURCE SNAPSHOT**  
**Date:** 2026-11-22  
**Reviewer:** Agent 5 (independent QA/security/architecture)  
**Target:** `D:\Aegis` only  
**Legacy reference:** `D:\SofiaDev\SofiaAdaLyra` (strictly read-only)  
**Scope:** Stage 1 hardening, Stage 2 diagnostics, Stage 3 Core runtime, Stage 5 module framework, registry lifecycle, and the security boundaries they touch.

**Provisional items remain unresolved and binding:** **A-04** (adult eligibility), **B-01** (final backup/host placement), **B-06** (backup retention/RPO/RTO). No adult capability is implemented or authorized. No production, installed, Live-verified, or Owner-accepted claim is made.

---

## 1. Outcome and approval statement

**I do not approve this stage.** The implementation changed substantially while it was being reviewed. A runtime crate, a host binary, a module registry, a diagnostics library, and registry lifecycle extensions appeared during the review window. By the late snapshot the workspace built and passed 143 tests, including 7 runtime tests with a real child process. However, the module/diagnostics/registry code still has no end-to-end CLI or IPC entry point, STOP is not enforced by the runtime host, and several lifecycle, authorization, read-only, and Stage 1 hardening gaps remain.

**Implementations that were not seen, not built, or not tested must not be approved unseen.** This review is explicitly a pre-integration risk review. It is not an approval of any agent's report, any design comment, any partially written file, or any test count quoted by another agent. Earlier snapshots of this tree failed to compile or failed tests; those counts are retained only to show volatility.

### 1.1 Commands actually observed

| Check | Command | Observed at snapshot |
|---|---|---|
| Format | `cargo fmt --all -- --check` | **Pass** at the late snapshot. |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | **Pass** at the late snapshot. |
| Workspace tests | `cargo test --workspace` | **143 passed, 0 failed** at the late snapshot: contracts 15, identity 18, install 12, integrity 13, registry 19, runtime 7, core-storage 12, diagnostics 8, IPC 5, modules 3+12, Platform 6, Safe 2+6, storage 5. |
| Runtime build | `cargo check -p aegis-core-runtime` | **Pass** at the late snapshot. Earlier in the review it failed with unresolved `rusqlite` and an `E0599` on `state_raw.clone()`. |
| Runtime tests | `cargo test -p aegis-core-runtime` | **7 passed**: real child start/stop, stale heartbeat, unauthorized mutation, unknown Core, one-Core failure isolation, retired-Core rejection, DB/manifest hash preservation. |
| Module tests | `cargo test -p aegis-modules` | **3 unit + 12 integration tests passed** at the late snapshot. An earlier attempt had 1 of 11 registry tests failing (`missing_dependencies_are_rejected`); the file changed during review. |
| CLI audit | Source inspection | No `diagnostics`, `runtime`, `core start/stop`, `core reactivate`, `core uninstall`, or `module` command was observed. |

The 143-test count is a late snapshot, not an inherited claim. It does not prove STOP enforcement, unresponsive-host termination, module authorization, diagnostics read-only behavior, or an end-to-end registry reactivation/uninstall command.

### 1.2 What exists versus what is proven

| Area | Exists as source | Proven by observed tests | Not proven |
|---|---|---|---|
| Core runtime | `aegis-core-runtime` + `aegis-core-host` binary | Build, real child start/stop, heartbeat freshness, stale heartbeat not running, owner denial, one-Core failure isolation, DB/manifest hash preservation | STOP enforcement, unresponsive-host forced kill/no orphan, lifecycle consistency for active/reactivated Cores, restart reconciliation, PID reuse, CLI/IPC |
| Module framework | Manifest, registry, 15 tests | Manifest parsing and registry lifecycle on synthetic fixtures | Owner authorization, CLI/IPC, install approval, permission grant model, dependency version enforcement, no-execution proof, boundary enforcement |
| Diagnostics | Library + 8 tests | Report shape and some tamper/missing cases | CLI/IPC, true read-only behavior, honest module/overall status, secret/private-content exclusion, path consistency |
| Registry lifecycle | Reactivate/uninstall/reconcile + 19 tests | Direct registry calls | Safe-boundary authorization, installer/CLI exposure, storage consistency, append-only enforcement, legacy owner policy |
| Stage 1 hardening | Batch 2 code and docs | Batch 2 baseline | TOCTOU, WAL/sidecar, checkpoint atomicity, permissions, manifest identity fields |

---

## 2. Files and evidence inspected

| Path | Observation |
|---|---|
| `crates/aegis-core-runtime/src/lib.rs` | Runtime state model, heartbeat thresholds, start/stop/status/list, owner authorization, lifecycle check. It compiles at the late snapshot and has 7 integration tests. |
| `crates/aegis-core-runtime/src/bin/aegis-core-host.rs` | Host verifies the Core DB once, writes heartbeats, exits on a stop sentinel. It does not read Safe STOP state. |
| `crates/aegis-core-runtime/Cargo.toml` | `rusqlite` is now present; the earlier late snapshot had a missing dependency and did not compile. |
| `crates/aegis-modules/src/manifest.rs` | Versioned manifest, strict field validation, SHA-256 self-digest. No signature/authenticity claim in the module docs. |
| `crates/aegis-modules/src/registry.rs` | Install/enable/disable/uninstall/verify/list/inspect/events. No owner-proof dependency or authorization call. Dependency versions are parsed but not enforced. |
| `crates/aegis-modules/tests/modules.rs` | Synthetic manifest/lifecycle/boundary tests. No owner-authorization, CLI, execution, or real package test. |
| `crates/aegis-diagnostics/src/lib.rs` | Aggregation library. Calls `PlatformService::start` and `SafeStore`, which initialize stores; computes Core path from `cores/<uuid>` rather than the registry path; uses global `SCHEMA_REVISION` as a fallback. |
| `crates/aegis-diagnostics/tests/diagnostics.rs` | Zero-Core, multi-Core, tamper, missing DB/manifest, STOP, module table cases. Does not hash before/after to prove read-only behavior. |
| `crates/aegis-core-registry/src/lib.rs` | Expanded lifecycle: deactivate, reactivate, uninstall-preserve, reconcile. Methods have no owner-token parameter. |
| `crates/aegis-core-registry/tests/registry.rs` | Direct registry tests for reactivation/uninstall/reconcile. Some tests set `active` by direct SQL. |
| `crates/aegis-core-install/src/lib.rs` | Installer gates initialize/rename/deactivate with owner proof. No reactivate/uninstall-preserve methods were observed. |
| `crates/aegis-cli/src/main.rs` | Core list/init/inspect/verify/rename/deactivate only. No diagnostics/runtime/module/reactivate/uninstall commands. |
| `crates/aegis-contracts/src/lib.rs` | Shared contract modules: core, initialization, integrity, IPC, lifecycle, roster, status. No runtime/module/diagnostics contracts. |
| `docs/completion/BLOCKERS.md` | Stage 1 high risks still listed: TOCTOU, checkpoint atomicity, WAL/sidecar, permissions, manifest identity, runtime supervision. |
| `docs/completion/MASTER_COMPLETION_LEDGER.md` | Runtime, activation/reactivation, uninstall-preserve, diagnostics, and modules were all `NS` in the ledger. The source has moved ahead of the ledger without a new acceptance record. |

---

## 3. Findings

Severity is pre-integration risk. "Open" means the orchestrator must resolve or verify it before treating any stage as integration-ready. Line references are to the snapshot observed; they may move as source changes.

| ID | Severity | Area | Finding / risk | Required check |
|---|---|---|---|---|
| **QA3-01** | **Resolved (late snapshot)** | Runtime build | `aegis-core-runtime` builds at the late snapshot. Earlier it failed with a missing `rusqlite` dependency and an `E0599`; both were fixed before the final test run. | Re-run `cargo check -p aegis-core-runtime` in the frozen review state; do not reuse the late count after further edits. |
| **QA3-02** | **Partially resolved** | Runtime tests | 7 runtime tests now pass, including a real child start/stop, stale heartbeat, unauthorized mutation, one-Core failure isolation, and DB/manifest hash preservation. There is still no STOP test, no unresponsive-host/forced-kill test, no orphan proof, no PID-reuse test, and no Platform-restart reconciliation test. | Add the missing runtime tests and assert OS process state, forced termination, no orphan, STOP activation, and restart reconciliation. |
| **QA3-03** | **High** | STOP enforcement | The host checks only `<root>/run/<uuid>.stop` (`aegis-core-host.rs:51-61`). It never reads Safe STOP. `CoreRuntime::start` does not check `safe.stop_status()`. A STOP that does not write the sentinel, or a host started before STOP, can keep running. | Enforce STOP at start and execution time. Test active STOP before start, STOP while running, STOP acknowledgement, and STOP failure visibility. |
| **QA3-04** | **High** | Runtime/registry split-brain | `require_runtime_lifecycle` allows only `installed` or `suspended` (`lib.rs:390-397`). `stop` calls the same check (`lib.rs:296-299`), so an `active` runtime cannot be stopped. `start` does not update the registry to active. `registry.reactivate` sets `suspended -> active`, so a reactivated Core may be unable to start under the runtime's current check. | Define one lifecycle-to-runtime mapping; make start/stop/reactivate consistent; add tests for installed, active, suspended, and retired Cores. |
| **QA3-05** | **High** | Orphan process | `stop` writes a sentinel and waits, but on timeout only marks failed; it does not kill the child (`lib.rs:296-340`). The child PID is recorded, but stop does not use it for forced termination. A host that ignores the sentinel or cannot write can remain alive after stop reports failure. | Track the runtime PID/instance and force-terminate after the stop timeout. Test an unresponsive host and prove the OS process is gone. |
| **QA3-06** | **Medium** | Stale recovery | `start` refuses to proceed when recorded state is live but stale (`lib.rs:198-200`) and offers no documented recovery or kill path. PID reuse is possible because the heartbeat binds only Core UUID and PID, not a process start time or runtime instance token. | Document stale recovery; bind heartbeat to runtime generation/instance and process start identity; test PID reuse and stale restart. |
| **QA3-07** | **Medium** | Heartbeat files | Heartbeats are written to a predictable temp file via `path.with_extension("heartbeat.tmp")` (`lib.rs:683-694`) with no containment/symlink re-check. `run_dir` is created under the root, but a redirected `run` directory is not explicitly rejected. | Canonical containment/no-follow checks for heartbeat/sentinel writes; random temp names; junction test. |
| **QA3-08** | **High** | Diagnostics is not read-only | The docs say diagnostics is a read-only observer, but `Diagnostics::new` calls `StoragePaths::new` (`lib.rs:197`), which can create the root and set permissions. `collect_platform` calls `PlatformService::start` (`lib.rs:292`), which calls `PlatformStore::initialize` and `SafeStore::start` -> `initialize`, potentially creating tables/rows/owner-proof paths. `collect_safe` opens initialized stores (`lib.rs:347-348`). | Use read-only SQLite opens or an immutable snapshot; hash/inventory before/after; prove no DB/table/file/audit creation on missing or partial roots. |
| **QA3-09** | **High** | Diagnostics honesty | When the module table is absent, `modules.status = unavailable` with a detail, but `overall = healthy` and `reasons` stays empty (`lib.rs:245-263`). The test explicitly asserts healthy. This may be defensible if modules are optional, but the status currently gives no reason a required module subsystem is unavailable. | Document whether modules are optional. If required, degrade/critical with a reason. If optional, state it in the report and acceptance policy. |
| **QA3-10** | **Medium** | Diagnostics path consistency | `collect_core` builds the DB path as `root/cores/<uuid>/core.db` (`lib.rs:423`) while also reporting `entry.storage_relative_path` (`lib.rs:450`). If the registry path differs, diagnostics checks a different file than the one registered. | Resolve the DB/manifest path through the same validated registry/storage resolver; test a registry path that differs from the UUID-derived path. |
| **QA3-11** | **Medium** | Diagnostics schema fallback | The fallback verification uses `aegis_storage::SCHEMA_REVISION` (`lib.rs:524`) when the Core snapshot cannot be read, rather than the actual Core manifest/revision. This can mislabel a schema mismatch. | Use the manifest's declared revision or report `unknown`; test a DB with a valid manifest but unreadable identity. |
| **QA3-12** | **Medium** | Diagnostics privacy | The report includes raw `stop_reason`, `stop_updated_by`, and error details (`lib.rs:96, 358`). Free-text reasons can contain private content. The "no secret material" doc claim is not tested against private text. | Bound/redact free-text fields; scan reports for private content and owner proof material; document the redaction policy. |
| **QA3-13** | **High** | Module authorization | `aegis-modules` has no `aegis-safe` dependency and no owner-token parameter. `install`, `enable`, `disable`, `uninstall`, and `verify` are public methods callable by any caller with Platform DB access (`registry.rs:171, 257, 320, 335, 380`). | Require Safe-boundary owner proof for every module mutation; add wrong/missing-token tests and no-side-effect assertions. |
| **QA3-14** | **High** | Module dependency semantics | Dependency version requirements are parsed (`manifest.rs` version requirement helper) but `ensure_dependencies_available` only checks that the dependency exists and is not failed/uninstalled (`registry.rs:536-570`). A consumer requiring `^1.0.0` can install against `0.1.0`. | Enforce version requirements or explicitly reject non-exact requirements in this stage; add a version-mismatch test. |
| **QA3-15** | **Medium** | Module enable policy | `enable` rejects a module with zero declared permissions (`registry.rs:264-267`, `NoDeclaredPermissions`). That may be intentional, but it makes a declaration list a hard enable gate and is not documented as policy. | Document the rule; test permissionless modules or change the gate. Do not confuse "declared permission" with "granted permission". |
| **QA3-16** | **Medium** | Module trust | `install` accepts a self-sealed manifest and does not require `expected_sha256`; the digest is content consistency only. There is no signature, publisher verification, install approval, or trust state. | Keep the no-authenticity wording; add an explicit install-approval/trust state before any execution stage; test that digest match is not called trusted. |
| **QA3-17** | **Medium** | Module digest canonicalization | `computed_digest` hashes a serde-serialized clone with the digest cleared (`manifest.rs:227-235`). Field order and serialization are tied to the current Rust struct; the "canonical form" is not a separately versioned byte contract. | Define a versioned canonical serialization or accept that digests are implementation-version-scoped; add a cross-version/round-trip test. |
| **QA3-18** | **Medium** | Module history | `aegis_module_events` has no database trigger preventing update/delete. Public API has no update/delete helper, but direct SQL can rewrite history. | Add triggers or an append-only enforcement test; verify a failed transition appends nothing. |
| **QA3-19** | **Medium** | Module integration | No CLI, IPC, Platform approval flow, or diagnostics deep integration exists for modules. Diagnostics only counts the `aegis_modules` table. A module cannot be installed/enabled through a supported user path. | Add the Stage 5 entry point and approval flow, or explicitly mark the stage library-only and not complete. |
| **QA3-20** | **High** | Stage 1 hardening | The high-severity Batch 2 blockers remain in `BLOCKERS.md`: TOCTOU between ownership check and writable open, non-atomic checkpoint replacement on Windows, WAL/sidecar hashing, permission hardening, and manifest identity gaps. No remediation was observed in the storage/identity/integrity files during this review. | Re-run the Batch 2 adversarial suite plus the HARD-01..HARD-06 gates; do not call Stage 1 hardened until they pass or are owner-accepted. |
| **QA3-21** | **Medium** | Shared contracts | `aegis-contracts` has no runtime, module, or diagnostics contracts. Runtime/module/diagnostics types live in their crates. Versioning and cross-crate compatibility are not centrally frozen. | Decide contract ownership and version policy; add schema files/tests if these are public contracts. |
| **QA3-22** | **High** | Registry authorization | `CoreRegistry::reactivate` (`lib.rs:249`), `uninstall_preserve` (`lib.rs:267`), and `reconcile`/`migrate_legacy_platform_core` (`lib.rs:322`) have no owner token. The installer exposes only initialize/rename/deactivate, so these new lifecycle operations are either direct-API-only or not exposed at all. | Require owner proof at the installer/service boundary; add CLI/command tests; do not accept direct registry calls as end-to-end evidence. |
| **QA3-23** | **Medium** | Registry history | The lifecycle history is append-only by convention. The table has no trigger, and tests do not attempt to update/delete a history row. | Add append-only enforcement or detection; test tampering and failed transitions. |
| **QA3-24** | **Medium** | Legacy reconciliation | `reconcile` imports legacy `platform_core` rows only when `core_registry` is empty, but it imports lifecycle states such as `active` without verifying the DB/manifest/storage path and without owner confirmation. It uses `INSERT OR IGNORE` and skips conflicts silently except for counts. | Require a report/owner-visible reconciliation; verify or quarantine imported storage; document whether legacy `active` is trusted or demoted. |
| **QA3-25** | **Low** | Runtime host executable | `resolve_host_executable` accepts `AEGIS_CORE_HOST` from the environment (`lib.rs:734+`). That is useful for tests, but it can execute an unapproved binary if the environment is poisoned. | Gate overrides to development/test mode; validate path/allowlist; document the production executable location. |
| **QA3-26** | **Low** | Diagnostics timestamp | `generated_at` uses `now_rfc3339().unwrap_or_default()` (`lib.rs:207`), so a clock error can produce an empty timestamp that still looks like a report. | Return/propagate a report-generation error or mark the timestamp invalid; do not accept an empty generated_at receipt. |
| **QA3-27** | **Medium** | Runtime observability | Runtime status has state/PID/heartbeat/last_error, but no structured event history and no link to registry lifecycle changes. A `running` runtime and a `retired` registry row can coexist without a reconciled event. | Add runtime transition events or a documented reconciliation; test registry/runtime divergence. |
| **QA3-28** | **Info** | Volatile source | Files changed during the review. A module test failed and then passed after the file changed. The CLI, installer, and contracts were not updated alongside the new crates. | Freeze a tree state/commit for review; re-run every gate against that exact state; do not reuse earlier counts. |

---

## 4. Focus-area analysis

### 4.1 Core runtime

The source is directionally aligned with the requested model: a real child host, heartbeat files, a stale threshold, an owner-token check, and a per-Core status. It is not acceptable yet because:

- It did not build at one snapshot; it now builds and has 7 tests, but the late result must be re-run in a frozen tree.
- STOP is a sentinel only; the host never checks Safe STOP.
- The lifecycle check makes an `active` runtime un-stoppable and a reactivated `active` Core un-startable.
- Stop timeout does not force-terminate or prove no orphan.
- PID reuse/instance identity, restart reconciliation, and a forced-kill test are untested. One-Core failure isolation is tested.

### 4.2 Module framework

The manifest validation is the strongest new code in the snapshot. It rejects unknown fields, reserved IDs, duplicate declarations, bad digests, unsafe paths, invalid SDK ranges, and invalid resource limits. The registry also preserves data on disable/uninstall and records events.

The gaps are security and integration, not schema shape: no owner authorization, no install-approval model, no permission grant model, no version-requirement enforcement, no append-only proof, no CLI/IPC, and no proof that a `process`/`native` entrypoint is never executed. The current `install` records a manifest; it must not be confused with "the module is trusted" or "the module may run".

### 4.3 Diagnostics

The report shape and per-Core isolation are useful. The two highest risks are honesty and read-only behavior:

- `PlatformService::start` and `SafeStore` initialization can create or mutate stores, which contradicts "read-only observer".
- `modules.unavailable` is treated as overall healthy with no reason, which may hide a missing Stage 5 subsystem.
- The Core path is derived from the UUID, not the registry's `storage_relative_path`, so diagnostics can check a different file than the registry points to.
- Free-text STOP reasons and errors can leak private content into the report.

### 4.4 Registry

Reactivation, uninstall-preserve, and conservative legacy reconciliation now exist at the registry layer. They are not acceptable as end-to-end features because:

- There is no owner authorization at the registry boundary.
- The installer/CLI do not expose reactivation or uninstall-preserve.
- The append-only history is convention, not enforcement.
- Legacy `active` rows can be imported without storage verification or owner confirmation.
- No test proves the registry path, actual DB/manifest, and identity are consistent.

### 4.5 Security boundaries

The intended boundaries are correct in the design documents: Platform owns registry/module metadata, Safe owns owner proof/STOP/audit, Core owns private data, modules are declarative in this stage, and owner proof is verified at the Safe boundary. The current source does not yet prove those boundaries:

- Module mutation has no Safe call.
- Diagnostics can initialize stores.
- Runtime STOP bypasses Safe.
- Runtime executable override is not gated.
- No test scans output for tokens, proof bytes, or private text.
- No test proves a module cannot access Core/Safe data because modules are not executable yet; the "no execution" property is a source-review claim that needs a test and a future execution gate.

---

## 5. Required checks before integration approval

These are minimum checks; the full matrix is in `STAGE_1_3_ACCEPTANCE_CRITERIA.md`.

### 5.1 Build, format, lint, test

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p aegis-core-runtime
cargo test -p aegis-core-runtime
cargo test -p aegis-modules
cargo test -p aegis-diagnostics
cargo test -p aegis-core-registry
```

Require the pinned Rust 1.88.0 toolchain. Record the exact tree state. Do not accept a test count from another agent. During this review the runtime build failed at one snapshot and passed at a later one, and a module test changed result while the file was edited; every gate must be re-run against a frozen tree.

### 5.2 Source review for dangerous patterns

```powershell
rg -n "remove_dir_all|remove_file|fs::rename|canonicalize|symlink_metadata|is_symlink|read_link|junction|reparse" crates/aegis-core-* crates/aegis-modules crates/aegis-diagnostics
rg -n "Command::new|spawn|kill|wait|stdin|stdout|stderr|AEGIS_CORE_HOST|CARGO_BIN_EXE" crates/aegis-core-runtime
rg -n "owner_token|verify_owner|authorize|Unauthorized|SafeStore|safe.stop" crates/aegis-core-runtime crates/aegis-modules crates/aegis-core-registry
rg -n "CREATE TABLE|INSERT INTO|UPDATE |DELETE FROM|ON CONFLICT" crates/aegis-modules crates/aegis-diagnostics crates/aegis-core-runtime crates/aegis-core-registry
rg -n "SHA-256|sha256|digest|authentic|trusted|signed" crates/aegis-modules crates/aegis-diagnostics
```

Inspect every destructive call, every child spawn, every path join whose input could be registry/CLI/IPC-controlled, and every mutation path missing an owner check.

### 5.3 Runtime evidence

- Build and locate `aegis-core-host`.
- Start two synthetic Cores on a disposable root; record PIDs and OS process existence.
- Kill a heartbeat and a child; assert `stale`/`failed`, not `running`.
- Stop with graceful and unresponsive hosts; assert forced termination and no orphan.
- Activate STOP before start and during run; assert denial/stop and visibility.
- Deny wrong/missing owner tokens with no spawn and no state change.
- Hash DB/manifest before/after; assert no data change and no instance-UUID rotation.
- Restart Platform; assert runtime rows reconcile without duplicate spawn.

### 5.4 Diagnostics evidence

- Hash/inventory the entire fixture root before/after diagnostics.
- Run zero-Core, multi-Core, tampered-Core, missing DB/manifest, missing/tampered Platform, missing/tampered Safe, active STOP, and module absent/present cases.
- Assert a reason for every non-healthy status and a documented overall-status rule.
- Assert `modules.unavailable` is either explicitly optional or causes the documented degraded/critical status.
- Scan the report for owner tokens, DPAPI blobs, proof bytes, and private text.

### 5.5 Module evidence

- Valid and invalid manifest fixtures: unknown fields, wrong schema, reserved IDs, traversal, duplicate capabilities/permissions/dependencies/assets/hooks, stale digest, expected-digest mismatch, missing/unavailable/version-mismatched dependency.
- Install/enable/disable/verify/uninstall; assert state, events, preserved data, and no execution.
- Owner-token denial for every mutation with no registry change.
- Prove no Core/Safe table is read or written and no child process is spawned.

### 5.6 Registry evidence

- Reactivate suspended; reject installed/active/retired.
- Uninstall-preserve from installed/suspended/active; assert retired state, history, and unchanged DB/manifest/path.
- Reconcile only from an empty registry; assert legacy rows untouched and skipped rows reported.
- Deny every mutation without owner proof.
- Search for destructive calls and prove no Core data deletion path exists.
- Assert the registry path resolves to the actual registered DB/manifest/identity.

### 5.7 Evidence audit

Reject:

- A pass claim without a reproducible command and tree state.
- A negative test that only asserts an error string and does not prove no side effect.
- A runtime claim based only on a unit test or a status row.
- A diagnostics "read-only" claim without before/after hashes.
- A module "no execution" claim without source review and a test.
- Any token, proof blob, private content, or secret in a report/evidence file.
- Any claim that resolves A-04, B-01, B-06, or asserts production/live/owner acceptance.

---

## 6. Open questions for the orchestrator

1. Which crate owns the frozen runtime contract (states, heartbeat, PID/instance identity, STOP, timeout), and where is it versioned?
2. What is the authoritative mapping between registry lifecycle (`installed`/`active`/`suspended`/`retired`) and runtime (`starting`/`running`/`stopping`/`stopped`/`failed`/`stale`)?
3. Does `start` require registry `installed` or `active`? Does a successful start update the registry? How does reactivation interact with start/stop?
4. How is STOP delivered to an already-running host without relying on the host's cooperation, and how is a STOP failure escalated?
5. What is the stale-process recovery policy: error, quarantine, forced kill, or owner-confirmed restart?
6. Is PID + Core UUID sufficient process identity, or is a runtime instance UUID/process start time required to reject PID reuse?
7. Where is the production `aegis-core-host` packaged, and is the `AEGIS_CORE_HOST` override development-only?
8. Should diagnostics open SQLite read-only, or may it use Platform/Safe service APIs that initialize stores? If the latter, how is read-only proven?
9. Is the module subsystem optional for overall health? If optional, how is that stated? If required, why is absent/unavailable still healthy?
10. Which module mutation boundary verifies owner proof: Platform service, installer, CLI, IPC, or the registry itself?
11. Are module dependency version requirements enforced, or explicitly unsupported in this stage?
12. What does "declared permission" mean before any grant model exists, and why does `enable` require at least one declaration?
13. Is the module manifest digest a stable canonical byte contract or an implementation-version-scoped convenience digest?
14. Is any module entrypoint executed in this stage? If not, what prevents a future enable/verify from executing it accidentally?
15. How is module uninstall data retention represented, and is `uninstalled` terminal or reversible?
16. Is uninstall-preserve a distinct state or a mapping to `retired`? Is that mapping frozen in contracts?
17. How are legacy `platform_core` lifecycle states reconciled, and is legacy `active` trusted as active or demoted?
18. How is append-only registry/module history enforced at the database layer, not just by convention?
19. Are runtime/module/diagnostics contracts shared in `aegis-contracts`, or intentionally crate-local? What is the version/deprecation policy?
20. Which Stage 1 blockers are fixed, which are deferred, and which have explicit owner acceptance with residual risk?
21. What is the tree state/commit for the final review, given the source changed during this review?
22. What evidence directory and owner signer will be used for B-07 acceptance?
23. Is Linux runtime/diagnostics behavior in scope for this stage, or explicitly Windows-only with a recorded limitation?
24. Which gates are allowed to be reported as Runtime-tested, and which remain Integration-tested only?

---

## 7. Provisional register and non-claims

| ID | Status | Binding interim rule |
|---|---|---|
| **A-04** | **PROVISIONAL - unresolved** | Adult capability remains 18+ and fail-closed; no eligibility bypass, model-only enforcement, or inferred eligibility. This stage does not implement or enable adult capability. |
| **B-01** | **PROVISIONAL - unresolved** | Final backup/Hephaestus target and Platform authority relocation placement are not settled. Do not assume co-location or a final host/process placement. |
| **B-06** | **PROVISIONAL - unresolved** | Retention/RPO/RTO are not settled. Do not claim a verified backup, a settled retention period, or a completed restore drill. |

No adult capability is implemented or authorized. No production, installed, Live-verified, or Owner-accepted claim is made. No legacy database, asset, service, or reference-repository file was opened writable or modified. No commit or push was performed.

---

## 8. Interim disposition

**NOT APPROVED - PRE-INTEGRATION.** The only acceptable disposition at this snapshot is to keep the stage open and require the actual source, a building runtime, real-process evidence, authorization at every mutation boundary, read-only diagnostics proof, and an independent re-review against a frozen tree state.

A future verdict may become "integration-ready with conditions" only after the rows in `STAGE_1_3_ACCEPTANCE_CRITERIA.md` have pass receipts and the open questions above are resolved or owner-accepted. It must not become "production", "Live-verified", or "Owner-accepted" without a named owner record.
