# Stage 1-3 / Stage 5 - Independent Acceptance Criteria

**Status:** Pre-integration acceptance contract. This document records criteria only. It is **not** an implementation approval, release claim, Live-verified claim, or Owner acceptance. It does not assert that any criterion has passed.

**Date:** 2026-11-22  
**Author:** Agent 5 (independent QA/security/architecture reviewer)  
**Target:** `D:\Aegis` only  
**Legacy reference:** `D:\SofiaDev\SofiaAdaLyra` (strictly read-only; no file modified)  
**Scope:** Stage 1 hardening, Stage 2 Platform diagnostics, Stage 3 Core runtime, Stage 5 module framework, Core registry lifecycle, and the security boundaries those stages touch.

**Provisional items remain unresolved and binding:** **A-04** (adult eligibility), **B-01** (final backup/host placement), **B-06** (backup retention/RPO/RTO). No criterion may settle them, assume them, or convert them into defaults.

**Non-claims:** No adult capability is implemented or authorized. No production, installed, Live-verified, or Owner-accepted claim is made. No Core is activated, started, or approved by this document.

---

## 1. Observed state at the time this contract was written

The source tree changed while this document was being written. The following is a snapshot, not a final state and not an approval. Agents' implementation reports were not trusted; the observations below come from reading the files and running the commands named.

| Area | Observed snapshot | Consequence for this matrix |
|---|---|---|
| Core runtime (`aegis-core-runtime`) | Source and the `aegis-core-host` binary appeared during review. An intermediate snapshot did not compile; by the late snapshot, `cargo check -p aegis-core-runtime` passed and 7 runtime tests passed, including a real child start/stop, stale heartbeat, unauthorized mutation, one-Core isolation, and DB/manifest hash preservation. No runtime CLI/IPC entry point and no STOP test were observed. | Start/stop and stale detection now have real-process evidence. STOP enforcement, forced termination/no-orphan behavior, runtime-to-registry lifecycle consistency, restart reconciliation, and CLI exposure remain open gates. |
| Core host (`aegis-core-host`) | A host loop writes heartbeats and exits on a `<uuid>.stop` sentinel. It verifies the Core database once at startup. It does **not** read Safe STOP state. | STOP, unresponsive-host termination, process-death cleanup, and PID-reuse handling remain explicit acceptance gates. |
| Module framework (`aegis-modules`) | `manifest.rs` and `registry.rs` exist. At the late snapshot, 3 manifest unit tests and 12 registry integration tests passed. No CLI, IPC, execution, sandbox, or install-approval flow was observed. The module registry has no owner-proof dependency or authorization call. | Manifest/lifecycle declarations are reviewable, but "no execution/data access yet" must be proven, not assumed. Module mutation authorization is not implemented at the registry boundary. |
| Diagnostics (`aegis-diagnostics`) | 8 integration tests passed at snapshot. No CLI command or IPC method exists. `collect()` calls `PlatformService::start`, which initializes Platform/Safe stores, so the documented "read-only observer" claim is not yet true for every path. Modules `unavailable` still yields overall `healthy` with no reason. | Diagnostics rows must prove read-only behavior and honest status, not only report shape. |
| Core registry (`aegis-core-registry`) | `reactivate`, `uninstall_preserve`, and legacy `reconcile` were added and 19 registry tests passed. The registry still has no owner-authorization parameter, and the installer/CLI were not observed to expose reactivation or uninstall-preserve. | Registry lifecycle rows require a Safe-boundary authorization check and an end-to-end entry point, not direct registry calls. |
| Stage 1 hardening | No observed remediation of the high-severity Batch 2 blockers in `BLOCKERS.md` (TOCTOU open, non-atomic checkpoint replacement, WAL/sidecar hashing, permission policy, manifest identity fields). File timestamps for storage/identity/integrity were unchanged during this review. | Stage 1 risks remain open and are carried as explicit gates below. |
| Baseline commands | At the late snapshot, `cargo fmt --all -- --check` passed, `cargo clippy --workspace --all-targets -- -D warnings` passed, and `cargo test --workspace` passed 143 tests: contracts 15, identity 18, install 12, integrity 13, registry 19, runtime 7, core-storage 12, diagnostics 8, IPC 5, modules 3+12, Platform 6, Safe 2+6, storage 5. An earlier snapshot had runtime compile failures and a transient module test failure. | The 143-test count is a late snapshot, not an inherited claim. Every acceptance receipt must name the exact tree state. |
| Git state | No commit exists; all project files are untracked. | There is no immutable reviewed revision. Evidence must include file hashes or a future commit ID. |

**Approval rule:** no implementation is approved by this document. Approval requires the actual source, a clean build, executed tests at the declared evidence class, retained receipts, resolved or owner-accepted findings, and a named Owner acceptance record (B-07). A design note, a test count from another agent, or a partially written file is not acceptance evidence.

---

## 2. Evidence classes and required receipts

| Class | Meaning | Minimum proof |
|---|---|---|
| **Designed** | A written design, contract, or specification exists. | Referenced document/type; no behavior claim. |
| **Coded** | Source exists in the target tree. | File/line reference. |
| **Built** | The code compiles in the pinned Rust 1.88.0 toolchain. | Exact command, exit code, timestamp. |
| **Unit-tested** | A test exercises a unit under the harness. | Test name, command, result. |
| **Integration-tested** | A test crosses crate/database/process-boundary behavior and asserts durable state. | Fixture, command, database/file assertions, result. |
| **Runtime-tested** | The built binary or child process was exercised on a disposable root. | Command transcript, exit code, OS process evidence, JSON output, hashes. |
| **Live-verified** | Verified in the intended installed/live environment. | Environment identity and runtime evidence. **Not available here.** |
| **Owner-accepted** | A named owner signed an acceptance record. | Signer, date, scope, evidence digest. **Not available here.** |

A unit test does not satisfy an Integration-tested row. A `cargo test` run does not satisfy a Runtime-tested row. Negative tests must prove both the expected denial **and** that no unintended side effect occurred.

Every accepted row must write a machine-readable receipt to `D:\Aegis\var\stage1-3-evidence\<ID>.json` or an owner-approved directory outside the worktree. Minimum fields: `test_id`, `declared_class`, `timestamp_utc`, `tree_state` (commit ID or SHA-256 of relevant source files), `host`, `command`, `fixture_root`, `preconditions`, `observed`, `hashes_before`, `hashes_after`, `negative_test`, `side_effects_checked`, `result` (`pass`/`fail`/`blocked`/`not-run`), `provisional_flags` (`A-04`, `B-01`, `B-06`), and `notes`. A missing receipt, a non-pass receipt, or a receipt without a reproducible command means the row is not accepted.

Fixture rules:

- Use a disposable absolute root outside the repository worktree.
- Use synthetic Core UUIDs, module IDs, and display names. Do not use a real Core's private data, legacy database, assets, or persona.
- Hash DB, manifest, heartbeat, sentinel, and sidecar files with SHA-256. Record sizes and timestamps.
- Do not delete pre-existing user data during a negative test.
- Do not use any result to settle A-04, B-01, or B-06.

---

## 3. Acceptance matrix

IDs are stable. "Min class" is the minimum acceptable evidence; higher evidence is better.

### 3.1 Core runtime

A runtime row is not accepted from registry state alone. It requires an observed OS process or an observed child-process exit.

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **RT-01** | Start creates a real child process. Evidence: recorded PID, OS process exists after start, PID differs from the caller, runtime state becomes `running` only after a fresh heartbeat carrying that PID and Core UUID. | A registry row or a stale/foreign heartbeat must never produce `running`; a heartbeat with the wrong PID or wrong Core UUID is rejected. | Runtime-tested |
| **RT-02** | Each start has a distinct runtime instance identity and a monotonically increasing generation. Evidence: runtime instance UUID/generation, start time, PID, and Core UUID in the status record; a second start while live is rejected without spawning another child. | Two live children for one Core must not be possible; a stale-generation start request is rejected and spawns nothing. | Integration-tested + Runtime-tested |
| **RT-03** | Heartbeat freshness is bounded and explicit. Evidence: heartbeat interval, stale threshold, observed heartbeat timestamps, computed age, and `heartbeat_fresh`/state in status. Timestamp parsing rejects malformed/future/non-monotonic values or reports a clock error. | A heartbeat older than the stale threshold is never fresh; a malformed or wrong-Core heartbeat is ignored, not trusted. | Runtime-tested |
| **RT-04** | Stale process detection works without relying on a registry flag. Evidence: suspend/kill the child or stop its heartbeat, wait past the threshold, and observe `stale` (not `running`) with reason and age. | A live-looking row with expired heartbeat must not be reported `running`; the system must not silently adopt a new process as the old one. | Runtime-tested |
| **RT-05** | Process exit and crash are observed. Evidence: exit status, transition to `failed`/`stopped`, no phantom `running`, and a clear error. | A child that exits before its first heartbeat, or dies later, must not leave a permanent `running` row; no auto-restart without explicit authority. | Runtime-tested |
| **RT-06** | Stop is real and bounded. Evidence: owner proof, stop request, graceful sentinel/wait, forced termination after `STOP_TIMEOUT_MS`, OS confirmation that the PID is gone, no orphan child, and idempotent repeated stop. | A child that ignores the sentinel must be terminated by policy; stop must not report success while the OS process still runs. Repeated stop must not fail destructively. | Runtime-tested |
| **RT-07** | Lifecycle and runtime state are consistent. Evidence: documented mapping between registry states (`installed`/`active`/`suspended`/`retired`) and runtime states (`starting`/`running`/`stopping`/`stopped`/`failed`/`stale`), plus tests for each transition. | `running` with registry `retired`, `active` with no runtime process, or a stop rejected solely because the registry says `active` is a failure; no split-brain status. | Integration-tested + Runtime-tested |
| **RT-08** | Owner authorization is required for start/stop/reactivate and any process-affecting action. Evidence: Safe-boundary token verification, denial code, no spawn/no state change on denial, and audit/trace without token content. | Missing, wrong, stale, or Core-originated token is denied; a Core worker cannot start or stop itself or another Core. | Integration-tested + Runtime-tested |
| **RT-09** | STOP is independently enforced at execution time. Evidence: with Safe STOP active, start is denied; a running host observes STOP (directly or through an authoritative signal written by Platform) and stops or becomes visibly failed; Platform does not depend on model compliance. | Enabling STOP while a Core runs must not leave it silently running; clearing STOP must not auto-start it without owner action. | Runtime-tested |
| **RT-10** | One-Core failure is isolated. Evidence: with A and B running, kill/corrupt/stale A; B remains running and heartbeat-fresh, Platform status remains available, and A is reported failed/stale. | A failure must not kill B, globally STOP, delete A/B data, or make B unreadable; A's private content must not appear in B's status or logs. | Runtime-tested |
| **RT-11** | Start/stop/crash preserves Core data. Evidence: DB, manifest, identity, and registry hashes before/after; database-instance UUID unchanged; no fresh DB or manifest regeneration. | A crash or stale stop must not rotate the instance UUID, delete sidecars, reseal the manifest without authorization, or create a second DB. | Integration-tested + Runtime-tested |
| **RT-12** | No hidden execution. Evidence: only explicit authorized start spawns a child; no auto-start at Platform boot; executable path/args are documented and validated; no shell; stdout/stderr do not leak secrets. | An environment override must not execute an unapproved arbitrary binary silently; a registry row alone must not execute anything; a Core cannot request "start me" as authority. | Runtime-tested + Source review |
| **RT-13** | Timeouts and recovery are defined. Evidence: first-heartbeat timeout, stop timeout, stale recovery action, retry limits, and whether forced kill/quarantine is required. | Timeout must not leave a child alive or a row permanently `starting`; stale recovery must not silently adopt a reused PID. | Runtime-tested |
| **RT-14** | Platform restart reconciles runtime state. Evidence: after Platform restart, live children are detected or reported stale; no duplicate spawn; runtime rows without processes are reconciled. | A restart must not show `running` for a dead process or spawn a second child for the same Core. | Integration-tested + Runtime-tested |

### 3.2 Module framework

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **MOD-01** | Manifest schema is versioned and strict. Evidence: exact schema ID, required fields, unknown-field rejection, size/length/count limits, and valid/invalid fixture results. | Unknown fields, wrong schema, missing required fields, oversized config/arrays, and invalid resource limits are rejected before any registry write. | Unit-tested + Integration-tested |
| **MOD-02** | Module identity and paths are constrained. Evidence: bounded lowercase module IDs, reserved IDs rejected, control characters/NUL/traversal rejected, entrypoint/asset paths relative and contained. | `aegis`, `aegis.*`, `platform`, `safe`, `core`, `..`, absolute/UNC/drive paths, empty components, reserved device names, and control characters are rejected without file creation. | Unit-tested |
| **MOD-03** | Duplicate declarations are rejected. Evidence: duplicate capability, permission, dependency, asset path, and lifecycle hook each fail with a specific error. | A manifest with two `log.write` permissions or two assets at the same path is not installed or partially stored. | Unit-tested + Integration-tested |
| **MOD-04** | Dependencies are validated. Evidence: missing dependency, unavailable (`failed`/`uninstalled`) dependency, self-dependency, duplicate dependency, and version-requirement behavior are tested. | A dependency requirement must not be silently ignored; an unavailable dependency must block install/enable; no partial registry row remains. | Unit-tested + Integration-tested |
| **MOD-05** | Manifest digest is validated. Evidence: lowercase 64-hex SHA-256, computed digest over the declared canonical form, declared digest equals computed digest, optional expected digest equals computed digest. | Uppercase/malformed/stale digest and expected-digest mismatch are rejected; a tampered manifest cannot verify. | Unit-tested + Integration-tested |
| **MOD-06** | Install records a validated manifest once. Evidence: record fields, installed state, manifest digest, event, duplicate-ID rejection, and no dependency partial write. | Reinstalling the same module ID must not overwrite or duplicate; a failed dependency check must leave no row/event. | Integration-tested |
| **MOD-07** | Enable/disable follow documented transitions. Evidence: legal transition matrix (`installed`/`disabled`/`failed` to `enabled`, `installed`/`enabled` to `disabled`), declared-permission requirement, and error for illegal transitions. | `uninstalled -> enabled`, duplicate enable, or enable without the declared-permission policy being satisfied is rejected without state change. | Integration-tested |
| **MOD-08** | Verify detects tampering. Evidence: stored manifest JSON mutated, digest mutated, malformed JSON, and invalid structure each produce `digest_mismatch`/`invalid_manifest`, not `verified`. | A consistent-looking but tampered manifest must not be reported as publisher-authentic or trusted; no auto-repair. | Integration-tested |
| **MOD-09** | Disable and uninstall preserve module data. Evidence: before/after file inventory and hashes for module data/manifest/events; lifecycle state only changes. | Uninstall must not delete module files or registry history by default; no recursive parent deletion. | Integration-tested |
| **MOD-10** | Permissions are declarations only in this stage. Evidence: enabling a module grants no Core/Platform/Safe data access, no network, no filesystem capability, and no token; the stored permission list is metadata. | A module with declared `core.read` or `secrets.read` must not receive that data merely by being enabled; no permission is inferred from capabilities or publisher. | Integration-tested + Source review |
| **MOD-11** | No module execution or loading exists yet. Evidence: install/enable/disable/uninstall/verify never execute entrypoints or lifecycle hooks and never spawn a process; no dynamic library/WASM loader exists. | A manifest with a `process`/`native` entrypoint and lifecycle hooks must be recorded without executing it; no child process or file write outside module data. | Integration-tested + Source review |
| **MOD-12** | Module registry is Platform-owned. Evidence: module tables live only in `platform.db`; no module table appears in Safe or Core DBs; no Core/Safe content is read or copied. | A module operation must not open a Core DB or Safe DB; no module private data is placed in Platform tables beyond declared metadata. | Integration-tested |
| **MOD-13** | Module lifecycle events are append-only and ordered. Evidence: event sequence for install/enable/disable/uninstall; no update/delete helper; event IDs monotonic. | Direct API must not rewrite history; a failed transition must not append a success event. | Integration-tested + Source review |
| **MOD-14** | Module mutations require owner/Platform authority. Evidence: Safe-boundary authorization for install, enable, disable, uninstall, and any approval step; wrong/missing token denial with no registry change. | No module mutation may be callable directly by a module or unauthenticated caller; a module cannot enable itself or grant itself permissions. | Integration-tested |
| **MOD-15** | Trust wording is honest. Evidence: docs/status distinguish content integrity from publisher authenticity; signed/unsigned state is explicit; no "trusted" or "safe" claim from digest alone. | A self-sealed manifest must not be reported as owner-approved or publisher-verified. | Documentation review + Unit-tested |

### 3.3 Diagnostics

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **DIA-01** | Zero-Core Platform is healthy. Evidence: `zero_core=true`, `core_count=0`, Platform/Safe available, STOP inactive, no Core rows, overall status `healthy`, reasons empty. | Absence of Cores must not be reported as a failure or cause an implicit Core/database to be created. | Integration-tested |
| **DIA-02** | Multi-Core diagnostics are per-Core and accurate. Evidence: registry count equals report count; each Core has path, bytes, lifecycle, integrity status; all verified yields healthy. | One missing Core path must not collapse the whole report or hide other Cores. | Integration-tested |
| **DIA-03** | Tamper/missing DB/missing manifest are reported per Core. Evidence: one tampered Core yields `degraded` with reason and expected/actual digest; other Cores remain `verified`; missing DB and missing manifest produce distinct statuses. | A tampered Core must not be reported verified; one bad Core must not delete/quarantine data or abort the report. | Integration-tested |
| **DIA-04** | Platform failure is critical and honest. Evidence: missing/tampered platform DB, malformed registry table, wrong application marker, or schema mismatch yields `critical`/unavailable with reason and no panic. | Diagnostics must not recreate or repair the Platform DB silently; it must not report healthy from a partial read. | Integration-tested |
| **DIA-05** | Safe failure is critical and honest. Evidence: missing/tampered Safe DB, missing owner proof, malformed STOP row, and audit-chain failure each yield explicit unavailable/error or critical status. | Missing owner proof must not be reported as healthy/available; diagnostics must not initialize or mutate Safe. | Integration-tested |
| **DIA-06** | STOP state is accurate. Evidence: inactive and active STOP snapshots, revision, updated_at/by/reason, and consistency between Platform and Safe sections. | Diagnostics must not clear/alter STOP; a stale STOP report must not claim inactive when Safe says active. | Integration-tested |
| **DIA-07** | Module availability is honest. Evidence: `available` with count when the module table exists; `unavailable` when absent; `error` on read failure. The overall status/reasons must explicitly say when a required module subsystem is unavailable. | A missing module table must not be presented as full health without an explicit unavailable reason or documented optional-subsystem policy. | Integration-tested + Documentation review |
| **DIA-08** | Overall status has a deterministic, documented rule. Evidence: mapping table from component failures to `healthy`/`degraded`/`critical`, and a reason for every non-healthy status. | `healthy` with any critical component unavailable, an unexplained degraded status, or an empty reasons list for a degraded report is rejected. | Integration-tested |
| **DIA-09** | Diagnostics is read-only. Evidence: SHA-256 and inventory before/after for Platform DB, Safe DB, Core DBs, manifests, and sidecars; no created files except the optional report/evidence output; read-only SQLite opens or a documented immutable snapshot. | Running diagnostics on a missing/corrupt root must not create DBs, tables, owner proof, heartbeat, module rows, or audit rows. | Integration-tested |
| **DIA-10** | Secrets and private content are excluded. Evidence: report/log scan for owner token, DPAPI blob, manifest secrets, private conversation text, and full free-text reasons; redaction policy. | A STOP reason or error containing private content must be redacted or bounded; no token or proof bytes in output. | Integration-tested + Source review |
| **DIA-11** | Report schema is stable and versioned. Evidence: schema ID, generated_at, sections, nullability, deterministic field names, and a JSON round-trip. | Unknown fields or unsupported schema must be handled by policy; a missing/empty timestamp must not silently produce an accepted receipt. | Unit-tested + Integration-tested |
| **DIA-12** | Diagnostics has a supported entry point. Evidence: a documented CLI/IPC method with exit codes and JSON output, or an explicit statement that library-only is not Stage 2 complete. | An undocumented library-only implementation must not be reported as a delivered diagnostics command. | Runtime-tested |

### 3.4 Core registry lifecycle

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **REG-01** | Reactivation is `suspended -> active` only. Evidence: owner proof, transition event, updated timestamp, active state, and preserved data. | `installed`, `active`, `retired`, or unknown states cannot reactivate; a retired Core is terminal for normal operation. | Integration-tested |
| **REG-02** | Reactivation preserves identity and data. Evidence: Core UUID, number, storage path, database-instance UUID, DB/manifest hashes unchanged. | No new DB, no new instance UUID, no manifest reseal, no storage move on reactivation. | Integration-tested |
| **REG-03** | Uninstall-preserve retires without deletion. Evidence: state `retired`, append-only event, DB/manifest/path/data hashes unchanged, preserved-path receipt. | No `remove_dir_all`/`remove_file`/truncate/move of Core data; no wildcard or parent-directory delete. | Integration-tested + Source review |
| **REG-04** | Lifecycle history is append-only. Evidence: ordered events with schema, Core UUID, from/to, reason, changed_at; one event per successful transition; no public update/delete API; database-level protection or documented detection. | A failed/illegal transition appends nothing; direct SQL cannot silently rewrite history without detection; duplicate/missing events are caught. | Integration-tested + Source review |
| **REG-05** | Legacy reconciliation is conservative and one-way. Evidence: only when `platform_core` exists and `core_registry` is empty; report counts imported/skipped; invalid/conflicting rows skipped; `platform_core` unchanged. | Reconciliation must not delete/rewrite legacy rows, import into a non-empty registry, or silently adopt invalid/foreign storage. | Integration-tested |
| **REG-06** | Legacy active/retired rows are handled honestly. Evidence: documented rule for importing legacy lifecycle state, owner-visible report, and no assumption that legacy `active` means a live runtime. | A legacy `active` row must not create a running process or bypass owner activation. | Integration-tested + Documentation review |
| **REG-07** | Duplicate identity is rejected. Evidence: duplicate Core UUID, Core number, storage path, and non-canonical UUID variants are rejected; canonical form is stored. | Uppercase/braced/simple UUID variants must not create a second row or DB; a reused path must not overwrite another Core. | Integration-tested |
| **REG-08** | Registry mutations require owner authority. Evidence: Safe-boundary check for register/rename/deactivate/reactivate/uninstall/reconcile; denial code and no state change on failure. | A direct registry call without owner proof, or a Core/module self-promotion request, must not mutate lifecycle state. | Integration-tested |
| **REG-09** | Registry/storage consistency is verified. Evidence: row path resolves to the expected Core DB/manifest/identity; mismatch is reported, not trusted. | A registry path pointing at another Core's DB, a missing DB, or a foreign manifest must fail closed. | Integration-tested |
| **REG-10** | Zero-registered versus preserved-data is visible. Evidence: status after uninstall-preserve shows zero registered/active Cores while listing preserved paths/data. | Status must not imply data was deleted when it was preserved, or that a retired Core is still running. | Integration-tested |
| **REG-11** | Lifecycle vocabulary is frozen or versioned. Evidence: mapping table for the six-state contract and any runtime/module/legacy terms; added states are versioned. | An undocumented `deactivated`/`uninstalled`/`running` string must not be persisted or silently coerced. | Documentation review + Unit-tested |
| **REG-12** | Registry mutations are isolated. Evidence: a corrupt/invalid one-Core row is reported per Core; other rows and Platform status remain usable. | One bad legacy/registry row must not abort startup or delete other rows. | Integration-tested |

### 3.5 Security and boundary controls

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **SEC-01** | Owner proof is required for every protected mutation. Evidence: runtime start/stop/reactivate, module install/enable/disable/uninstall, registry register/rename/deactivate/reactivate/uninstall/reconcile, STOP set/clear, and destructive confirmation all verify at the Safe boundary. | Missing/wrong token is denied before any child spawn, file operation, DB row, audit success, or state transition. | Integration-tested + Runtime-tested |
| **SEC-02** | Authorization failures have no side effects. Evidence: before/after inventory and hashes for denied operations. | A denied request must not create a heartbeat, sentinel, DB, row, event, or log containing the submitted token. | Integration-tested |
| **SEC-03** | No secret leakage. Evidence: scan stdout/stderr/logs/receipts/reports for owner token, DPAPI blob, proof bytes, private Core content, and credentials. | A development print-token flag, if present, requires an explicit warning and is never a default; no secret in evidence files. | Runtime-tested + Source review |
| **SEC-04** | Platform/Safe/Core/Module boundaries are preserved. Evidence: separate DB markers/files; no module/Core/Safe table in the wrong store; no direct Core DB access from modules or diagnostics writes. | A module or runtime must not open Safe/Core private data directly; diagnostics must not mutate Safe/Platform. | Integration-tested + Source review |
| **SEC-05** | No hidden execution or privilege escalation. Evidence: explicit start only; no auto-start at boot; no module entrypoint/hook execution; no shell; validated executable/args; no Core/model self-authority. | An unapproved `AEGIS_CORE_HOST` override, a Core "activate me" request, or a module enable must not execute code or grant authority. | Runtime-tested + Source review |
| **SEC-06** | STOP is independent of model/Core cooperation. Evidence: execution-time STOP check or authoritative signal, visible failure to acknowledge, and no model-mediated enforcement. | A model refusal or Core silence must not keep an effect running after STOP; STOP failure must be diagnosable. | Runtime-tested |
| **SEC-07** | Digest is not authenticity. Evidence: status wording and docs distinguish consistency from publisher/owner trust; consistent DB+manifest tamper is not called authentic. | No report/status/CLI may say "authentic", "trusted", "authorized", or "owner-approved" solely from a SHA-256 match. | Documentation review + Integration-tested |
| **SEC-08** | Destructive behavior is never a default. Evidence: uninstall/disable preserve data; purge requires separate owner proof/confirmation/tombstone; delete paths are canonical-contained. | A bare uninstall/disable must delete nothing; a wrong token must delete nothing; wildcard/parent deletes are rejected. | Integration-tested + Runtime-tested |
| **SEC-09** | Process and path inputs are constrained. Evidence: absolute disposable roots, root outside worktree or recorded override, canonical containment, no symlink/junction redirects, no traversal in module/entrypoint/child args. | A junction/symlink swap, `..` path, UNC/device path, or malicious child argument must not write/execute outside the root. | Integration-tested |
| **SEC-10** | Module permissions follow least privilege. Evidence: installation approval, runtime permissions, Core access, user-data access, host/network/filesystem access are separate; signed package is not automatically trusted. | A declared capability or publisher signature must not imply a permission grant; a module cannot access Core/Safe data or the network by default. | Integration-tested + Source review |
| **SEC-11** | Audit/evidence is content-free. Evidence: events/audit include action, actor, from/to, timestamp, result, and hashes, not tokens or private text. | Audit/receipt logs must not contain owner proof bytes, private conversation text, or full secret material. | Integration-tested + Documentation review |
| **SEC-12** | No live/owner claims from local tests. Evidence: every report labels local development vs installed/live/owner-accepted separately. | A passing workspace test or a local smoke run must not be described as Live-verified or Owner-accepted. | Documentation review |

### 3.6 Stage 1 hardening carried gates

These are carried from `BLOCKERS.md` and Batch 2 QA findings. They are prerequisites for treating Stage 1 as hardened.

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **HARD-01** | TOCTOU is closed or fail-closed. Evidence: open-once or revalidation on the writable handle; swap test between ownership check and writable open; no write to the swapped target. | A same-user swap between check and open must not write/read outside the intended Core DB; the operation fails closed or stays on the original inode. | Integration-tested |
| **HARD-02** | Manifest/marker containment is canonical. Evidence: write-time canonical containment/no-follow for manifest, marker, heartbeat, sentinel; junction at Core/init/run directory rejected. | A junction created after layout construction must not redirect a manifest/checkpoint/heartbeat write outside the root. | Integration-tested |
| **HARD-03** | Permissions are applied and tested. Evidence: documented directory/file modes or ACL policy for root, `cores/`, `init/`, `run/`, DB, manifest, proof; Unix mode tests and Windows ACL assumptions recorded. | A created Core directory/manifest must not be world-readable/writable where the policy forbids it; failures fail closed. | Integration-tested + Documentation review |
| **HARD-04** | WAL/sidecar hashing is defined and tested. Evidence: hash basis, journal mode, checkpoint/quiescence, `-wal`/`-shm` handling, concurrent-writer result. | Uncheckpointed WAL or a concurrent write must not produce a false `verified`; sidecars are inventoried and handled. | Integration-tested |
| **HARD-05** | Checkpoint replacement is crash-atomic. Evidence: atomic replace primitive, random temp name, no-follow creation, fault injection at replace. | A crash at replace must leave the old valid checkpoint or the new valid checkpoint, not neither; a predictable temp name must not be followed. | Integration-tested |
| **HARD-06** | Manifest/identity fields are complete. Evidence: `database_instance_uuid`, schema revision, ownership metadata, canonical UUID, and `database_file`/path comparison are represented and verified. | Missing/foreign instance/ownership/identity must fail closed; a directory rename or copied DB must not be adopted. | Integration-tested |

---

## 4. Top acceptance checks to run after integration

Run in order; stop on the first failure and retain the receipt.

1. **Tree and build gate**
   - Record the tree state; confirm only intended files changed.
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
   - `cargo check -p aegis-core-runtime` and build the `aegis-core-host` binary.
   - Every command must pass in the pinned Rust 1.88.0 toolchain. A previous pass count is invalid after source changes.

2. **Runtime real-process gate (RT-01 through RT-14)**
   - Install two synthetic Cores; start both with owner proof.
   - Assert two distinct OS PIDs, fresh heartbeats, correct Core UUID/PID binding, and `running` status.
   - Kill A's heartbeat or child; assert A becomes `stale`/`failed` while B stays running.
   - Stop A and B; assert graceful exit, forced-kill timeout behavior, and no orphan process.
   - Activate STOP; assert start is denied and a running Core stops or becomes visibly failed.
   - Hash DB/manifest before/after; assert no data change and no instance-UUID rotation.

3. **Module manifest and lifecycle gate (MOD-01 through MOD-15)**
   - Feed valid, unknown-field, wrong-schema, reserved-ID, duplicate, traversal, stale-digest, missing-dependency, unavailable-dependency, and version-mismatch fixtures.
   - Install, verify, enable, disable, uninstall; assert state, events, preserved data, and no execution.
   - Confirm no module can read Core/Safe data, spawn a process, or grant itself a permission.
   - Confirm all module mutations are owner-authorized and denied without a token.

4. **Diagnostics honesty and read-only gate (DIA-01 through DIA-12)**
   - Zero-Core, two-Core, tampered-Core, missing-DB/manifest, missing/tampered Platform, missing/tampered Safe, active STOP, and module table absent/present cases.
   - For each case, assert overall status and a reason for every non-healthy result.
   - Hash/inventory before/after; assert diagnostics creates or mutates nothing.
   - Scan output for tokens, proof bytes, and private text.

5. **Registry lifecycle and authorization gate (REG-01 through REG-12)**
   - Reactivate suspended, reject installed/active/retired reactivation.
   - Uninstall-preserve from installed/suspended/active; assert retired state, history, and unchanged data.
   - Reconcile only from an empty registry; assert legacy rows are untouched and skipped rows are reported.
   - Search for destructive calls; verify no Core DB/manifest deletion path exists.
   - Deny every mutation without owner proof.

6. **Security gate (SEC-01 through SEC-12)**
   - Wrong/missing/stale token for every protected operation.
   - Junction/symlink/traversal fixtures and child-process argument fixtures.
   - Secret scan of stdout/stderr/logs/receipts.
   - Confirm no hidden execution, no self-promotion, and no authenticity claim from a digest.

7. **Stage 1 regression gate (HARD-01 through HARD-06)**
   - Re-run the Batch 2 adversarial fixtures plus the new TOCTOU, WAL, checkpoint, permission, and manifest-identity fixtures.

8. **Evidence audit and independent re-review**
   - One receipt per ID with exact command, tree state, fixture, hashes, result, and side-effect checks.
   - Negative tests prove denial plus absence of side effects.
   - A-04/B-01/B-06 remain flagged.
   - Re-review the actual source diff, not agent summaries. This document is not an approval.

---

## 5. Exit criteria

This stage may be considered **integration-ready** only when all of the following hold:

- The workspace builds and the pinned-toolchain format/lint/test gates pass at a recorded tree state, including `aegis-core-runtime` and the `aegis-core-host` binary.
- Every intended row has a `pass` receipt at the declared class. Runtime rows have real OS-process evidence, not registry/status assertions alone.
- Core runtime has a documented lifecycle-to-runtime mapping, real start/stop, heartbeat freshness, stale detection, owner authorization, STOP enforcement, one-Core isolation, and data preservation.
- Module manifests reject malformed/duplicate/dependency/digest cases; install/enable/disable/verify/uninstall are tested; permissions are declarations only; no module execution or data access exists yet; all mutations are authorized.
- Diagnostics reports zero-Core and multi-Core honestly, isolates tamper/missing artifacts, reflects STOP and module availability, documents its overall-status rule, is truly read-only, and excludes secrets/private content.
- Registry reactivation, uninstall-preserve, append-only history, conservative legacy reconciliation, duplicate rejection, owner authorization, and no-destructive-deletion behavior are tested end to end.
- Security gates prove owner proof, no secret leakage, preserved Platform/Safe/Core/Module boundaries, no hidden execution, and no model/Core self-authorization.
- Stage 1 blockers are resolved or explicitly owner-accepted with residual risk recorded.
- A-04, B-01, and B-06 remain unresolved/provisional in every output.
- No adult capability, production, installed, Live-verified, or Owner-accepted claim is made.
