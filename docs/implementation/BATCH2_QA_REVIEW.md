# Batch 2 / Wave 1b - Independent QA, Security, and Architecture Review

**Review status:** **NOT APPROVED - POST-IMPLEMENTATION, PRE-INTEGRATION REVIEW**  
**Date:** 2026-11-22  
**Reviewer:** Agent 5 (independent QA/security/architecture)  
**Target:** `D:\Aegis` only  
**Reference:** `D:\SofiaDev\SofiaAdaLyra` (strictly read-only)  
**Scope:** Batch 1 baseline, shared contracts, and the actual Batch 2 source that landed during review. No Rust code, Cargo file, contract, or other file was modified by this review.

**Provisional items remain unresolved and binding:** **A-04** (adult eligibility), **B-01** (final backup/host placement), **B-06** (backup retention/RPO/RTO). No adult capability exists or is authorized. No production, installed, Live-verified, or Owner-accepted claim is made.

---

## 1. Outcome and approval statement

**I do not approve Batch 2.** The four Batch 2 crates were initially observed as doc-comment stubs, then were implemented concurrently while this review was being written. The final observed state includes actual source and tests, and I independently ran the workspace commands. The code is **Coded, Built, and per-crate Unit/Integration-tested**, but it is **not integration-ready** and is not accepted.

Independently executed on 2026-11-22:

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | **FAIL** (exit 1); diffs in `aegis-contracts/src/{core,integrity,roster}.rs`. |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | Pass (exit 0). |
| Tests | `cargo test --workspace` | **90 passed, 0 failed**. |
| CLI build | `cargo build -p aegis-cli` | Pass (exit 0). |

The passing tests are per-crate. There is **no cross-crate Batch 2 integration test**, and the actual source contains three critical integration contradictions:

1. **`core_identity` schema mismatch.** Storage creates the table without a `schema` column (`crates/aegis-core-storage/src/lib.rs:475-485`); integrity selects `schema` from it (`crates/aegis-core-integrity/src/lib.rs:481-482`). Integrity's unit-test fixture uses a different table shape, so the tests do not exercise a storage-created database. A real `verify_core(..., Some(identity))` will fail the stored-identity read and report `OwnershipMismatch`.
2. **Two registry tables.** Batch 1 `PlatformStore` creates and counts `platform_core` (`crates/aegis-platform/src/store.rs:39,87`); `aegis-core-registry` creates `core_registry` (`crates/aegis-core-registry/src/lib.rs:69`). Registering through the new registry does not update Platform status `core_count`; zero-Core status remains wrong and there are two sources of truth.
3. **No `database_instance_uuid` exists anywhere.** SPEC-13 requires it. Storage does not store it; contracts do not carry it.

Additional high-severity findings are in Section 4 (especially QA-26 through QA-36). `git log` still shows no commits; all files are untracked. This review is against the working tree, not a committed release.

Batch 1 is present and was inspected as the baseline. The workspace test run includes Batch 1 tests and passed, but that does not certify Batch 1 for production or owner acceptance.

---

## 2. Files and evidence inspected

| Path | Observation |
|---|---|
| `crates/aegis-contracts/src/core.rs` | `CoreIdentity`, `CoreStorageMetadata`, UUID/number/display-name validation. `CORE_STORAGE_SCHEMA = "aegis.core.storage.v1"` (line 13), not the SPEC-13 schema name. |
| `crates/aegis-contracts/src/initialization.rs` | Stage/status/failure vocabulary, checkpoint, report. Validation does not enforce stage/status consistency. |
| `crates/aegis-contracts/src/integrity.rs` | Manifest/report, SHA-256 validation, `IntegrityStatus`. No authenticity/signature field. `identity_sha256` is undefined in meaning. |
| `crates/aegis-contracts/src/roster.rs` | Registration request, registry entry, lifecycle change, `validate_storage_path`. Path checks are incomplete for Windows. |
| `crates/aegis-contracts/src/lifecycle.rs` | Six lifecycle states and approved/prohibited Core numbers. No transition-legality function. |
| `crates/aegis-core-identity/src/{lib,identity,state_machine,checkpoint,error}.rs`, `tests/identity_initialization.rs` | Pure identity validation, rename helper, six-stage state machine, JSON checkpoint persistence/recovery. 18 tests. |
| `crates/aegis-core-storage/src/lib.rs`, `tests/core_storage.rs` | Per-Core layout, UUID path normalization, fresh DB creation/reopen, ownership checks, rename, SQLite integrity. 12 tests. No database-instance UUID, no manifest/marker write, no permission hardening. |
| `crates/aegis-core-integrity/src/lib.rs` | SHA-256 file/identity hashing, manifest store, `seal_manifest`, `verify_core`, status mapping. 13 unit tests. No cross-crate test against storage-created DBs. |
| `crates/aegis-core-registry/src/lib.rs`, `tests/registry.rs` | `core_registry`/`core_lifecycle_change` tables in Platform DB, registration, display-name update, `installed|active -> suspended` deactivation, history. 9 tests. No Platform-status integration, no owner authorization, no uninstall/reactivation. |
| `crates/aegis-storage/src/lib.rs` | Batch 1 root canonicalization, symlink check only at root, SQLite application markers, schema table, `journal_mode = DELETE`, `SCHEMA_REVISION = 1`. |
| `crates/aegis-safe/src/{owner_proof,store}.rs` | DPAPI current-user proof on Windows, Unix 0600 fallback, Safe DB/STOP/audit. Partial-init and unkeyed-audit risks. |
| `crates/aegis-platform/src/{store,service}.rs` | Platform identity, `platform_core` table, zero-Core status, owner-authorized dispatch. |
| `crates/aegis-ipc/src/lib.rs` | Newline-framed JSON-RPC, `MAX_FRAME_BYTES`, owner token in envelope. |
| `crates/aegis-cli/src/main.rs` | Batch 1 CLI; no Batch 2 Core commands yet. |
| `D:\SofiaDev\SofiaAdaLyra\docs\specs\SPEC-03, SPEC-12, SPEC-13` | Identity/Constitution, ownership/retention, database/asset storage requirements. |
| `D:\SofiaDev\SofiaAdaLyra\docs\aegis-discovery-v0.2.0\06_OWNER_DECISIONS_LOCKED.md` | 29 settled, 3 provisional decisions and binding invariants. |

The Cargo dependency graph still shows boundary risks: `aegis-core-identity` is pure (no SQLite) and cannot persist identity itself; `aegis-core-integrity` has no `aegis-storage` dependency and therefore cannot enforce the shared root/path guard; `aegis-core-registry` has no `aegis-platform` dependency and therefore cannot update `platform_core`/Platform status through the existing API. These boundaries require an explicit integration layer that the current tree does not contain.

---

## 3. Baseline architecture that Batch 2 must preserve

- Contracts are authority-free. Callers must not gain authority merely by constructing a valid contract object.
- Platform DB, Safe DB, and each Core DB are separate stores. Platform owns registry/lifecycle metadata; Safe owns owner proof/STOP/audit; a Core owns its private data. No shared Core private DB.
- Owner proof is verified at the Safe boundary. The IPC envelope carries a token but does not authenticate it.
- The owner token is a development credential. DPAPI current-user protection is not production-grade owner authentication.
- Lifecycle authority is Platform/owner-controlled. `Installed != Activated`; no implicit promotion; no model self-authorization.
- A-04/B-01/B-06 remain provisional and must not be settled by implementation defaults.

---

## 4. Findings

Severity is pre-integration risk. "Open" means the orchestrator must resolve or verify it before accepting Batch 2.

| ID | Severity | Area | Finding / risk | Required check |
|---|---|---|---|---|
| **QA-01** | Resolved | Review basis | Final integrated review saw actual source. `cargo fmt --check`, clippy `-D warnings`, and **103** workspace tests pass at the remediation check. A cross-crate install test now exists. | See `## Final Integrated Review` and `### UUID Canonicalization Remediation`. |
| **QA-02** | High | Contract vs spec | `CORE_STORAGE_SCHEMA` is `aegis.core.storage.v1` (`core.rs:13`), while SPEC-13 requires `aegis.core.private_database.v1` (or approved successor). `CoreStorageMetadata` also lacks `database_instance_uuid`, `ownership_metadata`, `source_database`, and `integrity_status`. | Freeze one schema/version and add required fields before implementation diverges. |
| **QA-03** | High | Integrity vs authenticity | SHA-256 proves content consistency only. The manifest is unsigned and unkeyed; an attacker who can alter the DB can often alter the manifest too. No status distinguishes authenticity. | Define status semantics and user-facing wording; never report "authentic/trusted/owner-approved" from a digest. Add a test that consistently tampered DB+manifest is not called authentic. |
| **QA-04** | High | Hashing vs WAL | Batch 1 forces `journal_mode = DELETE` (`aegis-storage/src/lib.rs:109`), but Batch 2 may use WAL or leave `-wal`/`-shm`. Hashing the main file without checkpointing can produce a false `Verified`, and concurrent writers can tear the hash. | Require a documented hash basis, quiescence/exclusive snapshot, checkpoint/close, sidecar checks, and WAL-negative tests. |
| **QA-05** | Partially resolved | TOCTOU / links | `CoreStorageLayout` now checks root/`cores`/`init` for redirects (`aegis-core-storage/src/lib.rs:100-150`), `verify_database_file` canonical-contains the DB, and a Windows root junction was rejected in manual testing. Residual: `CoreDatabase::open` still does a read-only ownership check then a separate writable open, so a swap between checks is possible; `integrity_path`/marker paths are not canonical-contains checked; Windows core-dir junction tests are absent. | Add no-follow/open-once or a containment re-check after writable open; test a swap and a core-dir junction. |
| **QA-06** | High | Path traversal | `validate_storage_path` (`roster.rs:104-115`) rejects `..` substrings and `\\`/`:`, but allows `.` components, trailing dots/spaces, NUL/control characters, Windows reserved device names, and Unicode-normalization collisions. It does not prove canonical containment. | Replace/duplicate with a shared Windows-aware validator; test reserved names, trailing dot/space, `cores/./x`, NUL, normalization, and canonical containment. |
| **QA-07** | Partially resolved | Ownership substitution | Integrity now fails closed when the stored identity row is absent; storage checks the DB identity/UUID; installer test covers copied/tampered DB and cross-Core isolation. Residual: registry `register` does not verify the DB/manifest, and there is no single four-way cross-check at registration. | Add registration-time storage verification and a manifest `database_file`/path check. |
| **QA-08** | Partially resolved | Interrupted init | `CoreInstaller` now writes a checkpoint under `init/`, marks failures, and the interrupted-checkpoint test blocks silent recovery. Residual: the failed-retry path is not tested end-to-end; a failed run with an existing DB can be unrecoverable because the new request timestamp differs from the stored `created_at`; checkpoint replacement is non-atomic on Windows (`checkpoint.rs:117-135`). | Add fault injection after each store, a failed-retry test with an existing DB, and an atomic checkpoint replace. |
| **QA-09** | Partially resolved | Registry duplication | **Status-count defect fixed.** `PlatformStore::core_count` now reads `core_registry` when present (`aegis-platform/src/store.rs:80-99`), and the install test verifies `core_count=2`. Residual: both `platform_core` and `core_registry` tables still exist; legacy `platform_core` rows are ignored once `core_registry` exists, and there is no migration. | Keep the fallback/one-source rule documented, add a migration/legacy test, and avoid writing both tables. |
| **QA-10** | High | Lifecycle legality | Contracts contain only six states and `CoreLifecycleChange` with `from`/`to`; there is no transition-legality or generation check (`lifecycle.rs:12`, `roster.rs:81-100`). The master spec also lists a broader generic lifecycle including `Failed`, `Deactivated`, `Uninstalled`. | Freeze the six-state mapping or version an extension; enforce legal transitions, owner authority, and stale-generation rejection. |
| **QA-11** | Partially resolved | Mutable display name | **Installer path fixed.** `CoreInstaller::rename_display_name` updates the DB, reseals the manifest, verifies, then updates the registry; the test confirms `verify` remains `verified` after rename. Residual: the storage and registry APIs remain independently callable, and a crash between reseal and registry update can split the two stores. | Declare the installer as the only supported rename path; add a reconciliation/repair test for a crash between stores; do not let direct storage rename be used as acceptance evidence. |
| **QA-12** | **High (confirmed bug)** | UUID canonicalization | `validate_core_id` parses but does not normalize; `CoreIdentity` stores the caller's string. With CLI `core init A9A0A604-A4E7-4A88-A985-3F071E2C8E7D`, validation accepts, a Core DB is created, then `stored_identity != *identity` fails because the DB stores the canonical lowercase form. The CLI reports `Core is already initialized`, prints exit 1, leaves a failed marker, no registry entry, and a canonical retry also fails with `AlreadyInitialized`. | Canonicalize the UUID before identity construction/compare; add a test for uppercase/braced/simple forms; ensure a rejected form creates no DB/residue. |
| **QA-13** | Medium-High | Schema revision | `aegis-storage::SCHEMA_REVISION` is one global constant for Platform and Safe (`lib.rs:16`). Batch 1 DBs are already revision 1. Adding Batch 2 registry/Core columns without a per-component migration will either be rejected or require changing unrelated stores. | Introduce per-component revisions or an explicit migration; test old/new/unknown revisions and cross-component independence. |
| **QA-14** | Medium-High | Destructive deletion | Deactivation/uninstall must preserve data by default, but a naive registry implementation could call `remove_dir_all`/`remove_file`. There is no contract-level destructive-operation gate. | Search for destructive calls; require explicit owner-proof purge, tombstones, containment checks, and hash-before/after receipts. |
| **QA-15** | Medium | Store boundaries | `aegis-core-registry` is intended to manage Platform registry metadata but has no dependency on `aegis-platform`; `aegis-core-integrity` has no `aegis-storage` dependency and may duplicate path policy. | Define layer ownership: registry writes Platform DB only through a Platform-owned store; integrity uses the shared path/root policy; Safe remains untouched. |
| **QA-16** | Medium | DPAPI development proof | Windows DPAPI current-user protection is accessible to same-user malware; the CLI auto-loads the token from `owner.proof`; Unix fallback is an unencrypted 0600 file. There is no user-presence or rate limit. | Keep the dev-only label; do not claim production owner auth; test wrong-token denial and no token leakage in logs/CLI. |
| **QA-17** | Medium | Safe partial init | `SafeStore::initialize_owner` writes `owner.proof` before checking/inserting `safe_owner` and uses `let _ = remove_file` on failure (`store.rs:79-101`). A crash/orphan proof can leave `start()` accepting a file without a matching owner row. | Fault-inject after proof write; require orphan detection/reconciliation; do not silently adopt a proof file. |
| **QA-18** | Medium | Integrity fields | `IntegrityManifest` has `database_file` (basename), `database_sha256`, `database_bytes`, and an undefined `identity_sha256` (`integrity.rs:60-87`). It lacks database-instance UUID and ownership metadata. `OwnershipMismatch` exists in the status enum without ownership fields in the manifest. | Define `identity_sha256` bytes/scope, add instance/ownership fields, and ensure the manifest can actually detect an ownership mismatch. |
| **QA-19** | Medium | Init contract semantics | `InitializationCheckpoint`/`CoreInitializationReport` validation does not reject `Complete` with a `Failed` stage, `InProgress` with `completed_at`, or a `failure_code` inconsistent with the stage. No attempt ID/generation exists. | Add cross-field validation or enforce in the state machine; test every inconsistent combination. |
| **QA-20** | Medium | IPC/CLI compatibility | Batch 2 methods are not in `IpcMethod`; `PlatformService::dispatch` returns `unsupported_method` for unknown methods (`service.rs:167-173`). New methods need a compatibility plan. | Require additive method registration, protocol/version policy, owner-proof on every method, and Batch 1 client regression tests. |
| **QA-21** | Medium | Worktree location | SPEC-13 says per-Core DBs and backups are outside a Git worktree, but Batch 1 smoke data lives in `D:\Aegis\var` inside the worktree. A Batch 2 default could quietly continue this. | Require an external/disposable root for per-Core data or an explicit recorded owner override; test the rejection. |
| **QA-22** | Medium | Error taxonomies | Contracts and Batch 1 expose separate error enums (`CoreContractError`, `InitializationContractError`, `RosterContractError`, `IntegrityContractError`, `StorageError`, `PlatformError`, `ServiceError`). CLI/IPC code mapping may diverge. | Require one documented mapping to IPC/CLI error codes and tests for each negative path. |
| **QA-23** | Low-Medium | Path validator semantics | `validate_storage_path` rejects any string containing `..` even inside a harmless filename (over-strict) while allowing `.` components and reserved names (under-strict). | Use component-wise validation, not substring checks; test false positives and false negatives. |
| **QA-24** | Low | Hash serialization | `identity_sha256` has no canonical serialization rule (field order, line endings, JSON canonicalization). Cross-platform or future re-serialization could change the digest for the same identity. | Define exact bytes to hash and a versioned canonical form; test LF/CRLF isolation. |
| **QA-25** | Low | Timestamp/clock assumptions | Contracts only check timestamps are non-empty; they do not validate RFC3339/UTC. Clock errors can make ordering/receipts unreliable. | Validate timestamp format and clock errors; record UTC in receipts. |
| **QA-26** | Resolved | Storage/integrity schema | Storage now writes `schema`, and integrity reads it; the install test creates a real storage DB, seals it, and verifies `verified`. | Keep a cross-crate test in the acceptance suite. |
| **QA-27** | Partially resolved | Instance identity | Storage now generates/stores `database_instance_uuid`, exposes it in `CoreStorageMetadata`, and rejects instance == Core UUID. Residual: it is not in the integrity manifest or registry entry, and no restore/reopen behavior test exists. | Add instance UUID to the manifest/registry (or document why metadata-only is Wave 1b), and test fresh vs reopen vs restore. |
| **QA-28** | Resolved (installer path) | Integrity/rename | `CoreInstaller::rename_display_name` reseals the manifest after rename and verifies; the test passes. Residual: direct `CoreDatabase::rename_display_name` still leaves the manifest stale, and a crash between DB rename and reseal is not fault-injected. | Use the installer path as the only supported rename; add a crash/fault test and document direct storage API limits. |
| **QA-29** | Open (Medium) | Permissions | Installer starts with `StoragePaths` (root 0700 on Unix), but `cores/` and `init/` are created by `CoreStorageLayout` without explicit permissions; manifests are written via `NamedTempFile` without explicit 0600; Windows relies on inherited ACLs. | Apply a documented directory/file permission policy; test Unix modes and record Windows ACL assumptions. |
| **QA-30** | Open (High) | Link/path containment | `integrity_path` and `initialization_marker_path` still use only a string `starts_with` (`aegis-core-storage/src/lib.rs:181-211`); they do not canonicalize or reject a redirected Core/init directory. A manifest or checkpoint write can escape the root through a junction created after layout construction. | Canonicalize/re-check containment at write time; add a core-dir/init-dir junction test for manifest/checkpoint writes. |
| **QA-31** | Open (Medium) | Fresh/revision checks | `write_fresh_identity` writes fresh mode and the global revision, but `read_stored_identity` accepts any non-empty `initialization_mode` and any `schema_revision >= 1` (`aegis-core-storage/src/lib.rs:615-695`); `open` does not require fresh mode/current revision. | Validate `fresh_no_migration`, current revision, and instance UUID on open; test tampered rows. |
| **QA-32** | Partially resolved | Ownership skip | `verify_core` now returns `OwnershipMismatch` when `core_identity` is missing (`aegis-core-integrity/src/lib.rs:423-475`). Residual: it still does not compare `manifest.database_file` with the actual `database_path`, and it does not itself reject symlink/out-of-root paths; the caller must supply a safe path. | Add file-name and containment checks to verification; test wrong `database_file` and symlinked DB. |
| **QA-33** | Partially resolved | Registry path/auth | Installer gates `init`/`rename`/`deactivate` on owner proof and registers the expected `cores/<uuid>/core.db` path only after seal/verify. Residual: `CoreRegistry::register` still accepts any relative path and does not verify the DB/manifest; `deactivate` remains callable directly without owner auth; reserved-path comparisons are string/case sensitive. | Make the installer the only supported entry point and/or require storage verification/authorization inside the registry API. |
| **QA-34** | Medium-High | Checkpoint atomicity | `write_checkpoint` on Windows removes the destination then renames (`aegis-core-identity/src/checkpoint.rs:117-135`), creating a crash window with no checkpoint. The temp name is predictable (`.name.pid.counter.tmp`), and `File::create` follows links. | Use an atomic replace primitive, random temp names, no-follow creation, and a fault-injection test at the replace step. |
| **QA-35** | Resolved | Integration testing | `crates/aegis-core-install/tests/core_installation.rs` now covers storage + integrity + registry + Platform status, multi-Core isolation, repeated init, unauthorized init, rename/reseal, deactivation/hash preservation, tamper isolation, prohibited numbers, and read-only digest stability. | Keep this suite and add the residual fault/TOCTOU cases below. |
| **QA-36** | Resolved | Format gate | `cargo fmt --all -- --check` now exits 0. | Keep the gate in CI/acceptance. |
| **QA-37** | Open | Failure cleanup | `create_fresh` still removes only the DB file on some failure paths (`aegis-core-storage/src/lib.rs:369-387`), leaving `-wal`/`-shm`/directories; the uppercase-UUID bug proves an init failure can leave a DB + failed marker with no registry entry. | Add cleanup/quarantine and a retry/fault test; ensure a rejected UUID creates no DB. |
| **QA-38** | Open | Deactivation scope | Registry implements `installed|active -> suspended` only; no reactivation, retire, or uninstall-preserve command exists. The "active" test still reaches `active` by direct SQL. | Implement or explicitly defer these in the Wave 1b acceptance record; add uninstall-preserve receipts and a real activation path test. |

---

## 5. Focus-area analysis

### 5.1 Ownership validation

Ownership must be proven, not inferred. The required checks are:

- The Core DB identity row, the database file's parent UUID, the integrity manifest `core_uuid`, and the Platform registry `core_uuid` all agree.
- A missing ownership field is a fail-closed error, not an invitation to adopt the DB.
- A registry path is untrusted input; it must be resolved and contained, never used to open an arbitrary file.
- Cross-Core access is denied at the mediated API and at the storage path boundary. Direct SQLite access by a client is out of scope only if clients truly never receive a connection.

The current contracts contain ownership fields in several places but no cross-check function or result that says "the four sources disagree." `IntegrityStatus::OwnershipMismatch` exists, but `IntegrityManifest` has no ownership metadata, so that status cannot be derived from the manifest alone.

**Observed in the current source:** `verify_core` treats the stored-identity check as best effort and skips it when `core_identity` is absent (`aegis-core-integrity/src/lib.rs:423-465`). A self-consistent manifest and database with no identity table can therefore be reported `Verified`. The manifest `database_file` is also never compared with the actual `database_path`. Both must become hard failures when an expected identity is supplied.

### 5.2 TOCTOU, path traversal, junctions, and symlinks

Batch 1's root check is a good first step but not sufficient:

- `StoragePaths::new` checks the root after `create_dir_all` and before `canonicalize` (`aegis-storage/src/lib.rs:56-64`). It does not check intermediate components of later joins.
- `open_initialized` checks `symlink_metadata` on the final path when it already exists (`lib.rs:96-98`), but a symlink/junction created between the existence check and `Connection::open` can redirect the open.
- Windows junctions are reparse points; `file_type().is_symlink()` does not cover every reparse-point scenario. A path can also be swapped after validation.
- `SafeStore` reads `owner.proof` with `fs::read` after `is_file`; a link swap can redirect the read.

Required design: use a no-follow open where the platform permits it, canonicalize and re-check containment immediately before use, reject reparse points on every component under the root, and document the residual TOCTOU risk. Acceptance must include a swap test, not just a static symlink test.

**Observed in the current source:** `CoreStorageLayout::new` checks the root and `cores` directory and `verify_database_file` canonical-contains an existing DB, which is a good start. However `integrity_path`/`initialization_marker_path` only use a string `starts_with` (`aegis-core-storage/src/lib.rs:213-233`); they do not reject a symlinked/reparse-point Core directory. `create_fresh` also calls `create_dir_all` before re-checking the newly created directory, so a swapped `cores` junction can create a directory outside the root before the check fails. The only symlink test is `cfg(unix)`; Windows junction/reparse behavior is untested.

### 5.3 Database file hashing versus WAL

The integrity subsystem must define exactly what bytes it hashes. If SQLite is in WAL mode, the main `.db` file alone is not the logical database while `-wal` contains committed frames. A verifier that opens/hashes the main file while a writer is active can produce a digest that matches neither the before nor after state.

Required checks:

- Either close/checkpoint to a known journal mode and verify no `-wal`/`-shm` remains, or return a non-`Verified` status.
- Use an exclusive/read-consistent snapshot for verification, or fail closed when busy.
- Record the journal mode, hash basis, byte size, and whether sidecars were present.
- Test a WAL write without checkpoint, a concurrent writer, and a journal-mode change across versions.

**Observed in the current source:** `open_initialized` forces `journal_mode = DELETE` (`aegis-storage/src/lib.rs:109`), which reduces the WAL risk for normal opens, but integrity verification itself opens read-only and hashes the file after `PRAGMA integrity_check` without an exclusive lock or sidecar check (`aegis-core-integrity/src/lib.rs:319-365`). There are no WAL, sidecar, or concurrent-writer tests. A process that leaves a `-wal`/`-shm` pair or writes during verification is not covered.

### 5.4 Digest versus authenticity

A SHA-256 digest answers "do these bytes match this manifest?" It does not answer "who created this manifest?" or "is this Core allowed to run?" The manifest is currently unauthenticated. An attacker with write access to the Core directory can rewrite the DB and manifest together.

Required wording and behavior:

- Use "integrity consistent with manifest" or "digest matched" rather than "authentic", "trusted", or "owner-approved".
- If a status value is literally `Verified`, document that it means content consistency only.
- Do not use digest success as lifecycle authority, owner authorization, or package trust.
- Test consistent tampering of DB+manifest and confirm the verifier does not claim authenticity.

**Observed in the current source:** the integrity module explicitly documents "integrity is not authenticity" and says trust needs a separate authenticated channel (`aegis-core-integrity/src/lib.rs:1-20`). That is the right direction. The remaining risk is naming and evidence: `IntegrityStatus::Verified` can still be read as trust, there is no signature/MAC, and there is no test for a consistently tampered DB+manifest pair. Keep the documentation, and require user-facing text/tests that do not imply authenticity.

### 5.5 Interrupted/partial initialization residue

Init touches at least three durable artifacts: Core DB, integrity manifest, Platform registry row. A crash can occur between any two. There is no cross-store transaction. Likely residue:

- DB exists, manifest missing.
- DB+manifest exist, registry missing.
- Registry exists, DB/manifest missing.
- Temp file or `-wal`/`-shm` remains.
- Owner proof exists, Safe owner row missing (Batch 1 risk).

Required behavior: no residue is accepted as healthy or registered; every orphan is reported; retry either resumes from a validated durable checkpoint or fails closed; leftover files are quarantined, not silently deleted. Fault injection must run after every stage, not only at the end.

**Observed in the current source:** the identity state machine and checkpoint are reasonable in isolation (`aegis-core-identity/src/state_machine.rs`, `checkpoint.rs`), but they are not wired to storage, integrity, or registry, and `CoreInitializationReport` is unused. `write_checkpoint` removes the destination before rename on Windows (`checkpoint.rs:117-135`), so a crash in that window loses the previous checkpoint. `CoreDatabase::create_fresh` deletes the newly created DB on failure (`aegis-core-storage/src/lib.rs:320-330`) but leaves sidecars/directories. These are exactly the residue cases the integration suite must fault-inject.

### 5.6 Registry duplicates and split-brain

Batch 1 already owns a `platform_core` table with `core_uuid` primary key and a core-number CHECK (`aegis-platform/src/store.rs:39-46`). It has no display name, storage path, lifecycle generation, or unique core-number/path constraint. Batch 2's registry must decide:

- Is `aegis-core-registry` the domain layer over the existing Platform DB table, or does it own another table? A second table/DB is a split-brain risk.
- Are duplicate core numbers allowed? If not, add a unique constraint or an explicit policy and test.
- Are duplicate display names allowed? If yes, paths must not use display names.
- What happens to the existing Batch 1 database when the schema grows? A migration and revision bump are required; `CREATE TABLE IF NOT EXISTS` alone is not a migration.

**Observed in the current source:** both tables exist. `core_registry` has good uniqueness on UUID/number/path, but `PlatformStore::core_count` still counts `platform_core` (`aegis-platform/src/store.rs:87`). The registry's own tests never call `PlatformService::status`, so the split is invisible to the test suite. This is a confirmed zero-Core/status failure, not just a hypothetical.

### 5.7 Destructive deletion

Deactivation and uninstall default to preservation. The implementation must not perform recursive deletion of a Core directory or its parent. Explicit purge must require owner proof, separate confirmation, a content-free tombstone, and a deletion receipt; partial deletion must be reported incomplete. Delete paths must be canonical-contained and symlink-safe. Backup retention remains B-06 provisional; no test may claim a verified backup or settled retention.

### 5.8 Platform/Safe/Core separation

- Platform DB may hold registry/lifecycle metadata and should not become a second private Core DB.
- Safe holds owner proof, approvals, STOP, and security audit; it must not hold Core private content.
- Core DB must not contain `platform_identity`, `platform_core`, `safe_*`, or another Core's tables.
- The integrity manifest should live with the Core (or in an owner-approved integrity store), not in Safe and not in a way that lets Platform silently modify Core integrity evidence.
- The registry's storage path is metadata, not an authorization token. Platform ownership of the registry does not grant Platform access to Core private content.

### 5.9 DPAPI development-proof limitations

The Windows provider uses DPAPI current-user protection; the Unix fallback is a 0600 plaintext file. This is a development convenience, not production owner authentication. It does not protect against same-user malware, does not require user presence, and does not rate-limit token attempts. The CLI loads the token from the proof file, and Batch 1's `init --print-owner-token` can print it when asked. Batch 2 must not describe this as production-grade owner proof. Acceptance must test no-token-leakage and wrong-token denial, and evidence receipts must not contain the token.

Additional Safe concern: `initialize_owner` writes the proof file first, then checks/inserts the DB owner row (`aegis-safe/src/store.rs:79-101`). If the process dies after writing the proof but before the insert, a later `start()` sees the file and accepts it, while `initialize_owner` returns `OwnerAlreadyInitialized` because the file exists. That is a recoverable/repairable state today only by manual intervention; it should be explicitly detected and reconciled.

### 5.10 Cross-agent API conflicts

The contracts are not sufficient to pin the four agents to one API. The main conflicts are:

| Topic | What the contract currently expresses | Conflict / observed status | Required decision |
|---|---|---|---|
| Core DB schema | `CORE_STORAGE_SCHEMA = "aegis.core.storage.v1"` | SPEC-13 requires `aegis.core.private_database.v1` or approved successor. Storage actually writes the former. | Pick one name/version; add fields or document the smaller scope. |
| `core_identity` table | Contract has a `schema` field; storage table does not | **Confirmed:** storage omits `schema`; integrity selects it. | Freeze one table schema; add a cross-crate test. |
| Database instance UUID | Absent from all contracts and source | **Confirmed:** not generated/stored/verified. | Add to identity/metadata/manifest/registration; test. |
| Ownership metadata | Absent from `CoreStorageMetadata` and manifest | Integrity has `OwnershipMismatch` but can skip ownership when the table is absent. | Hard-fail missing ownership; compare manifest file/path/UUID. |
| Identity persistence | `CoreIdentity` is a plain struct; identity crate is pure | Storage owns the table; identity crate has no SQLite. | Declare the write path and transaction owner. |
| Display-name source of truth | Three copies (`CoreIdentity`, `CoreStorageMetadata`, `CoreRegistryEntry`) | **Confirmed:** rename updates one store and does not reseal the manifest. | Declare source + atomic sequence/reconciliation; test. |
| Lifecycle vocabulary | Six states in `lifecycle.rs` | Master spec also lists `Failed`, `Deactivated`, `Uninstalled`; registry maps deactivation to `suspended`. | Document the mapping or version an extension; add transition tests. |
| Generation/lease | `CoreLifecycleChange` has no generation | Not implemented; no stale-transition check. | Add generation or explicitly defer with a documented gap. |
| Registry table | Batch 1 creates `platform_core`; registry creates `core_registry` | **Confirmed:** two tables, two sources of truth; Platform status counts only the old one. | One table/sync; migration; integration test. |
| Schema revision | One global `SCHEMA_REVISION` in `aegis-storage` | Core/Platform/Safe share it; new registry component uses the same value. | Per-component revision or explicit migration plan. |
| Path policy | `validate_storage_path` in contracts; root check in `aegis-storage`; new `CoreStorageLayout` | Integrity has no `aegis-storage` dependency; storage does not use `StoragePaths` permissions. | Shared validator/resolver; one permission policy. |
| Core path layout | Storage uses `cores/<uuid>/core.db` | Registry accepts any relative path; no DB/manifest existence check at register. | Freeze the layout and verify it at registration. |
| Errors | Four contract errors + storage/platform/service errors | CLI/IPC codes not yet mapped for Batch 2 operations. | One mapping table and regression tests. |
| IPC methods | Batch 1 methods only | Batch 2 methods not in `IpcMethod`; unknown methods return `unsupported_method`. | Additive method registration and protocol compatibility policy. |
| Initialization report | `CoreInitializationReport` exists | Not produced by any Batch 2 implementation; state machine/checkpoint are isolated. | Wire the report into the orchestrator; test every stage/failure. |

### 5.11 Confirmed cross-crate defects and required order of remediation

These are the defects I would block on, in order:

1. **Unify `core_identity`.** Storage must write the `schema` column and the same columns integrity reads (or integrity must not read it). Add a test that creates a DB with `CoreDatabase::create_fresh`, seals it with `seal_manifest`, and verifies it with `verify_core(..., Some(identity))`.
2. **Unify the registry.** Choose `platform_core` or `core_registry`; make `PlatformService::status` read the chosen source. Add a test that registers a Core and asserts `zero_core=false`, `core_count=1`; then deactivates and asserts the preserved data.
3. **Add `database_instance_uuid`.** Generate it on fresh creation, store it in Core metadata, include it in the manifest/registry, and test reopen vs fresh vs restore.
4. **Fix rename integrity.** Decide whether `identity_sha256` covers immutable identity only or rename must reseal the manifest. Either way, test rename followed by `verify_core`. Keep the registry name in sync or declare reconciliation.
5. **Harden paths/permissions/integrity.** Add canonical containment/no-follow checks for manifest/marker writes, apply private permissions, make `verify_core` require an identity row and matching `database_file`, and add Windows junction tests.
6. **Wire init end-to-end.** Connect state machine/checkpoint to storage/manifest/registry, produce `CoreInitializationReport`, and fault-inject every stage. Make checkpoint replacement atomic on Windows.
7. **Complete lifecycle scope or declare it deferred.** Implement reactivation and uninstall-preserve, or explicitly mark those acceptance rows blocked/out of Wave 1b. Add owner authorization at the Platform service boundary.
8. **Add a cross-crate suite and fix formatting.** Run `cargo fmt --all`, then the full gate. No acceptance claim without the integration suite.

---

## 6. Required checks before integration approval

These are minimum checks; the full matrix is in `BATCH2_ACCEPTANCE_CRITERIA.md`.

### 6.1 Build, lint, and test

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Require the pinned Rust 1.88.0 toolchain. Do not accept a test count from another agent; inspect the output and receipts.

**Observed on 2026-11-22:** fmt **FAIL** (exit 1, diffs in contracts), clippy **PASS**, tests **90 passed / 0 failed**, `cargo build -p aegis-cli` **PASS**. The format failure must be fixed before a quality claim. Passing per-crate tests do not cover the cross-crate defects in Section 5.11.

### 6.2 Source review for dangerous patterns

```powershell
rg -n "remove_dir_all|remove_file|fs::rename|canonicalize|symlink_metadata|is_symlink|read_link|junction|reparse" crates/aegis-core-*
rg -n "join\(|storage_relative_path|display_name|core_uuid|database_instance_uuid" crates/aegis-core-*
rg -n "sha256|Sha256|wal_checkpoint|journal_mode|integrity_check|PRAGMA" crates/aegis-core-*
rg -n "core_identity|core_registry|platform_core|database_instance_uuid" crates/aegis-core-* crates/aegis-platform/src
rg -n "unsafe" crates/aegis-core-*
```

Inspect every destructive call and every path join whose input could be registry/CLI/IPC-controlled. Confirm no `unwrap`/`expect` on untrusted input in production paths. Specifically confirm `core_identity` is defined and read with the same columns in storage and integrity, and that only one registry table is authoritative.

### 6.3 SQLite and file evidence

For each fixture, capture:

- `PRAGMA application_id`
- `PRAGMA journal_mode`
- `PRAGMA integrity_check`
- `PRAGMA user_version` or the schema table revision
- `sqlite_master` table list
- identity/metadata/registry rows
- `Get-FileHash -Algorithm SHA256` for DB, manifest, `-wal`, `-shm`
- `Get-ChildItem -Force` for sidecars and staging residue
- canonical/real path and reparse-point status

### 6.4 Adversarial fixtures

- Same display name for two Cores.
- Same UUID in uppercase/braced/simple forms.
- Root and intermediate directory replaced by a symlink/junction.
- DB/manifest/owner proof final file replaced by a link.
- Registry path `cores/../../outside/core.db`, UNC, drive-relative, reserved device name.
- Missing/foreign ownership metadata.
- Copied DB from another Core.
- Fault after every init stage.
- Old/new/unknown schema revision.
- One-byte DB tamper; missing DB; missing manifest; malformed manifest; uppercase digest.
- WAL write without checkpoint; concurrent writer.
- Deactivate and uninstall with hash-before/after.
- Prohibited Core #4/#5 at library, CLI, and IPC entry points.
- Duplicate registry UUID/number/path.
- Wrong/missing owner token for every new IPC/CLI operation.
- Oversized/malformed IPC frames.
- Zero-Core startup and transition to one Core.

### 6.5 Evidence audit

Require a receipt per acceptance ID with command, environment, timestamp, result, hashes, and side-effect checks. Reject:

- A pass claim without a reproducible command.
- A negative test that only asserts an error string and does not prove no side effect.
- A "runtime" claim based only on a unit test.
- Any token, DPAPI blob, private Core content, or secret in logs/evidence.
- Any claim that resolves A-04, B-01, B-06, or asserts production/live/owner acceptance.

### 6.6 Required cross-crate Batch 2 suite

The orchestrator must add (not infer) a test that exercises the real seams:

1. `CoreStorageLayout::new` → `CoreDatabase::create_fresh` → `seal_manifest` → `verify_core(..., Some(identity))` → `PlatformService`/`CoreRegistry` registration → `PlatformService::status` shows one Core.
2. Rename through each public path, then `verify_core` and registry read.
3. Deactivate/suspend and uninstall-preserve, with DB/manifest hashes before/after and Platform status.
4. Fault injection after each init stage, including a crash between Core DB, manifest, and registry writes.
5. Corrupt/foreign DB and missing identity table, asserting `OwnershipMismatch`/`DatabaseMissing` rather than `Verified`.
6. Windows junction/reparse tests, not only `cfg(unix)` symlinks.

---

## 7. Open questions for the orchestrator

1. Which Core DB schema identifier is authoritative: `aegis.core.storage.v1` or `aegis.core.private_database.v1`?
2. Where is `database_instance_uuid` generated, stored, and exposed? Which contract owns it?
3. Is the integrity manifest signed or MACed? If not, what exact wording and threat model prevent digest/authenticity confusion?
4. What exactly does `identity_sha256` hash, and is it over immutable identity fields only? How does a display-name rename avoid invalidating integrity?
5. What is the single source of truth for display name, and how are the registry and Core DB reconciled after a crash?
6. Are duplicate Core numbers allowed? If not, where is the unique constraint and migration?
7. Should `deactivated`/`uninstalled` be added to the lifecycle contract, or mapped to `suspended`/`retired`? What version bump applies?
8. Does `aegis-core-registry` write the existing `platform_core` table through `aegis-platform`, or own a new table/DB? Who migrates Batch 1 databases?
9. How are per-component schema revisions handled given the single `SCHEMA_REVISION` constant in `aegis-storage`?
10. Is WAL allowed? If so, what is the checkpoint/quiescence rule for integrity hashing?
11. Where do Core DBs and manifests live relative to the Git worktree, and is there an owner-approved override?
12. What is the Core path layout and manifest location, and which module is the only path resolver?
13. What are the Batch 2 IPC method names, schemas, and protocol version policy?
14. What is the CLI syntax and exit-code/error-code contract for Batch 2 operations?
15. How are partial Safe owner-proof initialization and orphan proofs reconciled?
16. What is the policy for quarantining corrupt/foreign Core DBs, and what retention applies (without settling B-06)?
17. Does integrity verification require exclusive access or a copy/snapshot? How is a busy DB reported?
18. Are Core databases encrypted at rest in Wave 1b? (Batch 1 defers encryption; do not imply it exists.)
19. Which owner signer and evidence directory will be used for B-07 acceptance?
20. Which parts of SPEC-13's full Core composition are in Wave 1b versus explicitly deferred?
21. Who owns the canonical `core_identity` table shape, and can storage and integrity share one schema definition?
22. Which registry table is authoritative, and how is existing Batch 1 `platform_core` data migrated or retired?
23. Is rename a Core-DB-only operation, a registry-only operation, or a coordinated operation with manifest reseal? What is the crash recovery rule?
24. Are reactivation, retire, and uninstall-preserve in Wave 1b, or explicitly blocked/deferred?
25. Does any Wave 1b default place Core DBs inside the Git worktree, and what owner override is required?

---

## 8. Provisional register and non-claims

| ID | Status | Binding interim rule for Batch 2 |
|---|---|---|
| **A-04** | **PROVISIONAL - unresolved** | Adult capability remains 18+ and fail-closed; no eligibility bypass, model-only enforcement, or inferred eligibility. Batch 2 must not implement or enable adult capability. |
| **B-01** | **PROVISIONAL - unresolved** | Final backup/Hephaestus target and Platform authority relocation placement are not settled. Do not assume co-location or a final host/process placement. |
| **B-06** | **PROVISIONAL - unresolved** | Retention/RPO/RTO are not settled. Do not claim a verified backup or a settled retention period; restore drills remain required. |

No adult capability is implemented or authorized. No production, installed, live-verified, or owner-accepted claim is made. No legacy database, asset, service, or reference-repository file was opened writable or modified.

---

## 9. Interim disposition (superseded)

**Superseded by the final integrated review below.** This earlier pre-integration disposition applied before the final integration fixes. It is retained for audit history only; the final section controls the current verdict.

---

## Final Integrated Review (2026-11-22)

**Verdict: APPROVE WITH CONDITIONS for Wave 1b documentation completion. NOT approved for production, Live-verified, or Owner-accepted use.**

This verdict is based on the actual integrated source in `D:\Aegis`, not on the agents' reports. I inspected the contracts, storage, identity, integrity, registry, install orchestration, CLI, Batch 1 crates, and their tests. I independently ran the full gate and a disposable-root CLI smoke test. I did not modify Rust code, Cargo files, contracts, or any file outside the two allowed documentation files.

### 1. Test, format, and lint results actually observed

| Check | Command | Observed |
|---|---|---|
| Format | `cargo fmt --all -- --check` | **PASS** (exit 0). The earlier format failure is fixed. |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** (exit 0). |
| Tests | `cargo test --workspace` | **100 passed, 0 failed** at the initial final review; **103 passed, 0 failed** after the UUID remediation (see subsection). |
| CLI build | `cargo build -p aegis-cli` | **PASS** (exit 0). |

The initial 100-test count was accurate at that point; after the UUID remediation it is 103 (contracts 15, identity 18, install 12, integrity 13, registry 9, core-storage 12, IPC 5, Platform 6, Safe 8, Batch 1 storage 5). The install suite now includes a genuine cross-crate test (storage + integrity + registry + Platform status), so the earlier "no integration test" finding is resolved for the main path.

**CLI runtime smoke (disposable root under `%TEMP`, not the repository):** `init` (no token printed) -> `status` (`zero_core=true`, `core_count=0`) -> `core init` -> `core list` -> `status` (`zero_core=false`, `core_count=1`) -> `core inspect` -> `core verify` (`status=verified`) -> `core rename` (integrity re-verified) -> `core deactivate` (suspended) -> `core verify` (still verified) -> duplicate `core init` (exit 1, "Core is already initialized"). No `-wal`/`-shm` files remained. A junction used as the CLI root was rejected by `StoragePaths`.

### 2. Verified fixes

- `core_identity` now contains `schema`, `database_instance_uuid`, and the identity fields; storage writes them and integrity reads `schema`. The install test creates a real storage DB, seals it, and verifies it as `verified`.
- `CoreStorageMetadata` includes `database_instance_uuid`; storage rejects an instance UUID equal to the Core UUID; the install test asserts independence.
- Integrity now returns `OwnershipMismatch` when the stored identity row is missing, rather than silently skipping ownership.
- Platform `core_count` reads `core_registry` when that table exists; the install test confirms `core_count=2`, and CLI `status` confirmed `core_count=1` after registration.
- Initialization checkpoints live under `init/<uuid>.initialization.marker`, outside the Core DB directory; the marker was observed with `stage=registered`.
- `CoreInstaller` requires owner proof for `initialize`, `rename`, and `deactivate`; unauthorized init is denied without a DB, and orphan init residue is refused.
- `CoreInstaller::rename_display_name` reseals the manifest and verifies after rename; the test and CLI smoke both kept `status=verified`.
- Deactivation preserves DB and manifest hashes and leaves the Core verifiable.
- The CLI exposes `core list/init/inspect/verify/deactivate/rename`; errors go to stderr and return exit 1. Read-only inspection does not invalidate the digest (test `read_snapshot_does_not_invalidate_digest`).
- Tampered Core A is `DigestMismatch`; Core B remains healthy and both remain listed; #4/#5 are rejected.

### 3. Remaining risks and conditions

**High**

1. **Non-canonical UUID accepted, then leaves unrecoverable residue.** Confirmed by CLI: `core init A9A0A604-A4E7-4A88-A985-3F071E2C8E7D` creates a canonical Core DB, then fails because `stored_identity != *identity` (stored UUID is lowercase, request identity is uppercase). It returns `Core is already initialized`, exit 1, leaves a `failed` marker and an unregistered DB; a canonical retry also fails with `AlreadyInitialized`, and `core list` is empty. Condition: canonicalize the UUID before identity construction/comparison (or reject non-canonical input before creating anything) and add an uppercase/braced/simple-form test plus residue cleanup.
2. **TOCTOU between ownership check and writable open.** `CoreDatabase::open` performs a read-only identity check, then opens the same path writable in a separate operation (`aegis-core-storage/src/lib.rs:404-430`). A same-user swap between checks is still possible. Condition: open once with the required flags, or revalidate identity/ownership on the writable handle; add a swap test.
3. **Manifest/marker path containment is string-only.** `integrity_path` and `initialization_marker_path` use `starts_with` but do not canonicalize/reject a redirected Core/init directory (`aegis-core-storage/src/lib.rs:181-211`). A junction created after layout construction can redirect a manifest or checkpoint write. Condition: canonical containment/no-follow checks at write time and a Windows core-dir/init-dir junction test.
4. **Failed-init retry with an existing DB is not proven recoverable.** If `CoreDatabase::create_fresh` returns an existing DB from a failed run, the new request's `requested_at` becomes a different `created_at`; the `stored_identity != *identity` check then returns `AlreadyInitialized`. The existing interrupted test has no DB, so this path is untested. Condition: test a failed run with an existing DB and define resume-vs-quarantine behavior; ensure the marker/error explains the required action.
5. **Checkpoint replacement is non-atomic on Windows.** `write_checkpoint` removes the destination before rename (`aegis-core-identity/src/checkpoint.rs:117-135`), leaving a crash window with no checkpoint. Condition: use an atomic replace primitive and fault-inject the replace step.

**Medium**

6. **Permissions are not explicitly applied to `cores/`, `init/`, or manifests.** `StoragePaths` protects the root on Unix, but `CoreStorageLayout` creates the subdirectories without explicit modes and manifests use `NamedTempFile` defaults; Windows relies on inherited ACLs. Condition: apply and test a documented directory/file permission policy.
7. **Integrity manifest does not carry `database_instance_uuid`.** The metadata now does, but the manifest and registry do not; SPEC-13's backup-manifest requirement is not fully represented. Condition: add it or explicitly declare this a Wave 1b limitation.
8. **No WAL/sidecar/concurrent-writer verification test.** The normal open forces `journal_mode=DELETE`, and the CLI run left no sidecars, but `verify_core` has no sidecar/checkpoint/exclusive-snapshot test, and `database_file` is still not compared with the actual path. Condition: add WAL/sidecar/concurrent tests and the file-name check.
9. **Uninstall-preserve and reactivation are not implemented.** Registry only supports `installed|active -> suspended`; there is no reactivation, retire, or uninstall command, and the "active" test reaches `active` via direct SQL. Condition: implement or explicitly mark these Wave 1b-deferred in the acceptance record; add uninstall-preserve receipts when implemented.
10. **Registry API remains weaker than the installer.** `CoreRegistry::register` accepts any relative path and does not verify the DB/manifest; `deactivate` has no owner-auth parameter; the installer gates the CLI path. Condition: make the installer the only supported entry point or harden the registry API.
11. **Two registry tables remain.** `platform_core` and `core_registry` both exist; `core_count` uses `core_registry` when present, so legacy `platform_core` rows are ignored after registry initialization. Batch 1 was zero-Core, so no live data is known to be lost, but the migration rule is undocumented. Condition: document the one-source rule or migrate/retire `platform_core`.

**Low / documentation**

12. CLI exit codes are only 0/1; usage text still says "Batch 1"; the CLI auto-loads the development owner token from the proof file. This is consistent with the Batch 1 dev model but not production auth.
13. `CORE_STORAGE_SCHEMA` remains `aegis.core.storage.v1` rather than SPEC-13's `aegis.core.private_database.v1`; `source_database`/ownership metadata are still not in the storage contract.
14. `validate_storage_path` still allows `.` components, trailing dots/spaces, reserved Windows names, and case variants; registry reserved-path comparison is string-based.
15. No Batch 2 IPC methods exist; `core` operations are CLI-only. Batch 1 IPC remains compatible.
16. Integrity is still content integrity, not authenticity: no signature/MAC, and a consistent DB+manifest tamper is not detectable. The module documentation correctly states this; do not claim authenticity.

### 4. Corrections to earlier findings

- **Resolved:** QA-26 (`core_identity` schema), QA-32 (missing identity row now fails), QA-35 (cross-crate install test), QA-36 (format gate).
- **Partially resolved:** QA-09 (status count fixed, two tables remain), QA-11 (installer reseal fixed, direct APIs/crash window remain), QA-27 (instance UUID in metadata, not manifest/registry), QA-28 (installer reseal fixed, direct storage rename remains), QA-33 (installer auth/verification, registry API remains weak).
- **Confirmed and escalated:** QA-12 (UUID canonicalization) is now a reproduced High bug with residue, not just a risk.

### 5. Exact residual limitations and non-claims

- This is Wave 1b only: Core init, identity, per-Core DB, integrity manifest, registry, deactivation, and CLI. It is not a full Core runtime.
- No uninstall-preserve, reactivation, retire, encryption at rest, OS service/daemon, backup/restore drill, cross-host authority, or Batch 2 IPC API is implemented or claimed.
- No Live verification and no Owner acceptance. No named owner signer/acceptance record was supplied.
- **A-04** (adult eligibility) remains unresolved and fail-closed; no adult capability is implemented or authorized.
- **B-01** (final backup/host placement) remains unresolved; no co-location or final host placement is assumed.
- **B-06** (retention/RPO/RTO) remains unresolved; no verified backup or settled retention period is claimed.
- No legacy database, asset, service, or reference-repository file was opened writable or modified. No commit or push was performed.

### 6. Conditions for converting this verdict to full Wave 1b documentation acceptance

1. Fix canonical UUID handling and the residue case; add tests for uppercase/braced/simple forms and failed retries.
2. Add TOCTOU/path-containment tests for open, manifest, and checkpoint writes, including Windows junctions.
3. Fault-inject init at every store boundary and make checkpoint replacement atomic on Windows.
4. Apply and test the Core directory/manifest permission policy.
5. Add WAL/sidecar/concurrent-writer integrity tests and compare `database_file` with the actual path.
6. Implement or explicitly defer uninstall-preserve/reactivation in the acceptance record; do not imply they exist.
7. Resolve the `database_instance_uuid` manifest/registry question and the storage-schema/spec naming question.
8. Keep `cargo fmt --check`, clippy, and the current full suite green (103 tests at this remediation check), and retain a new test for each residual condition above.

Until those conditions are met, the correct status is **Wave 1b documentation-complete / integration-approved with conditions**, not production-ready, Live-verified, or Owner-accepted.

### UUID Canonicalization Remediation

**Status: the reproduced High issue is resolved for the normal CLI/installer path.**

I inspected the actual changes:

- `CoreIdentity::new` now calls `canonicalize_core_id` before constructing the identity (`aegis-contracts/src/core.rs:48-62`).
- `with_display_name` canonicalizes the stored UUID again (`core.rs:82-96`).
- `canonicalize_core_id` parses via `validate_core_id` and returns the hyphenated lowercase UUID (`core.rs:136-138`).
- The contract regression test `uuid_is_canonicalized_without_changing_identity` covers uppercase input through rename.
- The installer test `uppercase_uuid_is_canonicalized_and_does_not_leave_partial_state` initializes with uppercase and verifies with lowercase.
- The installer now compares stored/requested identity by UUID, Core number, and display name, not by `created_at`, and uses the stored identity for sealing/registration (`aegis-core-install/src/lib.rs:169-183`). This fixes the failed-checkpoint retry path that previously tripped on a new request timestamp.
- `failed_checkpoint_with_existing_database_can_be_recovered` covers a failed checkpoint with an existing DB.

**Commands actually observed (2026-11-22):**

| Command | Result |
|---|---|
| `cargo test -p aegis-contracts -p aegis-core-install` | **PASS**: 15 contract tests + 12 installer tests, 0 failed |
| `cargo test --workspace` | **PASS**: **103 passed, 0 failed** (the earlier 100 count is now stale; two tests were added) |
| `cargo fmt --all -- --check` | **PASS** (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **PASS** (exit 0) |
| `cargo build -p aegis-cli` | **PASS** |
| CLI fresh-root smoke | Uppercase `A9A0A604-A4E7-4A88-A985-3F071E2C8E7D` init **exit 0**; stored/returned UUID is lowercase; `core verify` with lowercase **exit 0**; `core list` shows the canonical UUID |

The previous uppercase CLI failure/residue does **not** reproduce on a fresh root after this change.

**Residual risks (not regressions of this fix):**

- `CoreIdentity` fields remain public and `Deserialize` is derived, so a caller can still construct a non-canonical identity without `new`; `validate()` accepts it. The normal installer path is canonical, but the type does not enforce canonical form at every boundary. Low-to-medium.
- A crash after the registry insert but before the final `registered` checkpoint can still leave a registered Core with a `failed`/interrupted marker; the new recovery test covers a DB-without-registry residue, not that registry-present residue. Medium.
- Pre-fix residue on an existing root is not auto-migrated or cleaned; it requires an explicit repair/quarantine path. Low.
- The fix does not address the separate TOCTOU/path-containment or checkpoint-atomicity findings.

No code, Cargo, contract, or other file was modified by this check. No commit or push was performed. Provisional A-04, B-01, and B-06 remain unresolved; no production, Live-verified, or Owner-accepted claim is made.
