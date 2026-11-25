# Batch 2 / Wave 1b - Independent Acceptance Criteria

**Status:** Pre-integration acceptance contract. This document records criteria only; it is not an implementation approval, release claim, or owner acceptance. It records independently executed build/test evidence separately from acceptance.

**Date:** 2026-11-22  
**Author:** Agent 5 (independent QA/security/architecture reviewer)  
**Target:** `D:\Aegis` only  
**Reference:** `D:\SofiaDev\SofiaAdaLyra` (strictly read-only)  
**Governing inputs:** `docs/specs/SPEC-03`, `docs/specs/SPEC-12`, `docs/specs/SPEC-13`, `AEGIS_MASTER_SPECIFICATION_v0.3.0.md`, and `06_OWNER_DECISIONS_LOCKED.md`.

**Provisional items remain unresolved and binding:** **A-04** (adult eligibility), **B-01** (final backup/host placement), **B-06** (backup retention/RPO/RTO). No test in this document may settle them, assume them, or turn them into defaults.

**Non-claims:** No adult capability is implemented or authorized. No production, installed, live-verified, or owner-accepted claim is made. No Core is created or activated by this document.

---

## 1. Observed state at the time of this review

This matrix is deliberately independent of the four implementation agents' reports. It was written after inspecting the actual files. The four Batch 2 crates were initially observed as stubs and were implemented concurrently while this review was being written; the final observed state below includes that source and an independent workspace test run.

| Area | What was observed | Consequence for this matrix |
|---|---|---|
| Shared contracts | `crates/aegis-contracts/src/{core,initialization,integrity,roster,lifecycle}.rs` contain contract types and contract-level unit tests. | Contract shape is reviewable; contract gaps are listed in the QA review. |
| Batch 2 crates | `aegis-core-identity`, `aegis-core-storage`, `aegis-core-integrity`, and `aegis-core-registry` now contain implementations and tests. Files include `aegis-core-identity/src/{checkpoint,error,identity,state_machine}.rs`, `aegis-core-storage/src/lib.rs`, `aegis-core-integrity/src/lib.rs`, `aegis-core-registry/src/lib.rs`, plus per-crate integration tests. | The rows below are testable against real code, but the workspace has **no cross-crate Batch 2 integration test**. Several critical integration gaps remain (see Section 1.2). |
| Batch 1 | `aegis-storage`, `aegis-safe`, `aegis-platform`, `aegis-ipc`, and `aegis-cli` contain working implementations and tests. | Batch 1 is the baseline whose boundaries Batch 2 must preserve. The workspace test run includes Batch 1 tests and passed. |
| Git state | `git log` shows no commits; all files are untracked. | No commit or release state exists to approve. |
| Reference repository | Read-only specs and locked decisions were inspected. | No reference file was modified. |

**Approval rule:** no implementation is approved by this document. Approval requires actual source, a clean build, executed tests, retained acceptance evidence, and a named owner acceptance record (B-07). Passing a build or test alone is not owner acceptance.

### 1.1 Independently executed evidence (2026-11-22)

| Check | Command | Observed result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | **FAIL** (exit 1). Diffs in `aegis-contracts/src/core.rs` (lines 78, 158), `aegis-contracts/src/integrity.rs` (lines 74, 116), and `aegis-contracts/src/roster.rs` (line 92). |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | Pass (exit 0). |
| Unit/integration tests | `cargo test --workspace` | **90 passed, 0 failed** (14 contracts, 18 core-identity, 13 core-integrity, 9 core-registry, 12 core-storage, 5 IPC, 6 Platform, 8 Safe, 5 storage). |
| CLI build | `cargo build -p aegis-cli` | Pass (exit 0). |

This is evidence of **Coded, Built, Unit-tested, and per-crate Integration-tested** code. It is **not** evidence of end-to-end Batch 2 integration, CLI/IPC Batch 2 runtime behavior, Live verification, or Owner acceptance. The format gate failing alone blocks a release-quality claim.

### 1.2 Critical integration gaps observed in the current source

These are current-code findings, not hypothetical risks. The orchestrator must resolve them before treating the passing per-crate tests as Batch 2 acceptance:

1. **Storage and integrity disagree on the `core_identity` table shape.** `aegis-core-storage` creates `core_identity` without a `schema` column (`crates/aegis-core-storage/src/lib.rs:475`), while `aegis-core-integrity` selects `schema` from that table (`crates/aegis-core-integrity/src/lib.rs:481`). The integrity unit tests use a different fixture schema, so they pass without exercising a real storage-created database. A real `verify_core(..., expected_identity)` will fail the stored-identity read and report `OwnershipMismatch`.
2. **Two Platform registry tables.** Batch 1 `PlatformStore` creates and counts `platform_core` (`crates/aegis-platform/src/store.rs:39,87`); `aegis-core-registry` creates `core_registry` (`crates/aegis-core-registry/src/lib.rs:69`). Registering a Core through the new registry does **not** change `PlatformService::status()` `core_count`, so zero-Core status remains wrong and there is no single registry source of truth.
3. **No `database_instance_uuid` is stored anywhere.** SPEC-13 requires it. Storage writes only `core_identity` with `schema_revision` and `initialization_mode`; integrity/registry contracts do not carry it.
4. **Rename breaks integrity and splits the display-name source of truth.** `rename_display_name` updates only the Core DB (`aegis-core-storage/src/lib.rs:383-400`); `update_display_name` updates only `core_registry` (`aegis-core-registry/src/lib.rs:190-207`); neither reseals the integrity manifest. Because `compute_identity_sha256` hashes `display_name` and `identity_revision`, the next integrity verification with the new identity is `OwnershipMismatch`.
5. **Initialization is not end-to-end.** The identity state machine is not connected to storage, integrity, or registry; no `CoreInitializationReport` is produced; no marker/manifest is written by the storage path; and repeated/interrupted init is not exercised across the three stores.
6. **Deactivation exists only as `installed|active -> suspended` in the registry.** There is no reactivation, retire, or uninstall-preserve implementation, and the registry methods have no owner-authorization parameter. The Platform status does not see these changes.
7. **Path/permission hardening is incomplete.** `CoreStorageLayout` does not use `StoragePaths` and does not set 0700/0600 permissions for the Core root, Core directories, or manifests. `integrity_path`/`initialization_marker_path` only do a string-prefix check, with no canonical/symlink re-check. Windows junction/reparse tests are absent (the only symlink test is `cfg(unix)`).
8. **Integrity can skip ownership.** If `core_identity` is absent, `verify_core` skips the stored-identity check and can still return `Verified`; `database_file` is not compared with the actual supplied path; there is no WAL/concurrent-writer test.
9. **Registry paths are not tied to the Core layout.** `register` accepts any relative path that passes `validate_storage_path` (for example `anything/core.db`) and does not verify that the DB/manifest exists or matches the UUID. Reserved-path comparison is case/trailing-dot sensitive on Windows.
10. **No cross-crate integration test exists.** Workspace tests are per-crate; the critical gaps above are exactly what a Batch 2 integration suite must exercise.

---

## 2. Evidence classes and how to classify a result

Every acceptance row must carry exactly one evidence class. Classes are cumulative, not interchangeable.

| Class | Meaning | Minimum proof |
|---|---|---|
| **Designed** | A written design/spec/contract exists. | Referenced document or contract type. |
| **Coded** | Source code exists in the target tree. | File/line reference. |
| **Built** | The code compiles in the pinned toolchain. | Exact `cargo` command and successful exit code. |
| **Unit-tested** | A test exercises a unit under the test harness. | Test name, command, result. |
| **Integration-tested** | A test crosses crate/storage/process-boundary behavior and asserts durable state. | Test name, command, fixture, database/file assertions, result. |
| **Runtime-tested** | The built binary/protocol was exercised as a running process on a disposable root. | Command transcript, exit code, JSON output, file/hash evidence. |
| **Live-verified** | Verified against the intended installed/live environment. | Environment identity, install receipt, runtime evidence. **Not available in Batch 2.** |
| **Owner-accepted** | A named owner signed an acceptance record. | Signer, date, scope, evidence digest. **Not available in Batch 2.** |

A row must not be marked Integration-tested merely because a unit test exists. A row must not be marked Runtime-tested merely because `cargo test` ran. Negative tests must assert both the expected failure **and** that no unintended side effect occurred.

### 2.1 Required evidence receipt for every row

Each test writes a machine-readable receipt to `D:\Aegis\var\batch2-evidence\<TEST_ID>.json` (or an owner-approved evidence directory outside the Git worktree). Minimum shape:

```json
{
  "test_id": "STO-01",
  "declared_class": "Integration-tested",
  "timestamp_utc": "2026-11-22T00:00:00Z",
  "host": "Windows 11 / PowerShell 5.1 / rustc 1.88.0",
  "command": "cargo test -p aegis-core-storage <exact_test_name> -- --exact --nocapture",
  "fixture_root": "C:\\Users\\<user>\\AppData\\Local\\Temp\\aegis-batch2-STO-01",
  "fixture_core_uuids": ["...", "..."],
  "preconditions": {"files": [], "hashes": {}, "registry_rows": []},
  "observed": {
    "exit_code": 0,
    "status": "pass",
    "error_code": null,
    "database_paths": [],
    "sqlite": {},
    "files": []
  },
  "hashes": {"before": {}, "after": {}},
  "negative_test": false,
  "side_effects_checked": [],
  "artifacts": [],
  "result": "pass",
  "provisional_flags": ["A-04", "B-01", "B-06"],
  "notes": ""
}
```

`result` is one of `pass`, `fail`, `blocked`, or `not-run`. A missing receipt, a receipt with `result != pass`, or a receipt without a reproducible command means the row is not accepted.

### 2.2 Required fixture rules

- Use a disposable absolute root outside the repository worktree. A relative root must be rejected.
- Use synthetic, non-production Core UUIDs and display names. Do not use a real Core's private data, legacy database, assets, or persona.
- Hash files with `Get-FileHash -Algorithm SHA256 -LiteralPath <path>` and record the exact bytes/size. Record `-wal` and `-shm` sidecars too.
- Do not delete pre-existing user data during negative tests. Use copies or disposable fixtures.
- Do not use a test result to settle A-04, B-01, or B-06.

---

## 3. Acceptance matrix

The IDs are stable. The orchestrator should require one receipt per ID (or a documented mapping from several tests to one ID). "Min class" is the minimum acceptable evidence class; higher is better.

### 3.1 Storage isolation and multiple Cores

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **STO-01** | Two Cores created under one configured root have two distinct database files, distinct database-instance UUIDs, and distinct Core ownership metadata. Evidence: absolute paths, `PRAGMA application_id`, `sqlite_master` table lists, identity rows, and a path-to-UUID map. | Opening Core A's database with Core B's identity/registry path fails with `OwnershipMismatch`/`ForeignOwnership`; no row is written to either database; both file hashes are unchanged. | Integration-tested |
| **STO-02** | Each Core database owns only its own private tables. Core A cannot read or write Core B's private tables through any public API or mediated contract. Evidence: table ownership list, denial result, audit/receipt, unchanged hashes. | A direct cross-Core open, attach, registry path substitution, or API call is denied; no data from B appears in A's logs/errors; no audit record leaks B's private content. | Integration-tested |
| **STO-03** | Platform DB, Safe DB, and every Core DB remain physically and logically separate. Core DB has no `platform_identity`, `platform_core`, `safe_stop`, `safe_owner`, or `safe_audit` tables. Platform DB has no Core private memory/conversation/relationship tables. | Attempting to create or read a Safe/Platform-owned table through the Core storage API is rejected; attempting to create a Core private table in Platform/Safe is rejected or not exposed. | Integration-tested |
| **STO-04** | Three or more Cores can be registered and listed independently. Status/registry reports exactly the registered count, and each entry resolves to its own existing database. Evidence: registry rows, count, per-Core integrity status, per-Core path existence. | Removing/corrupting one Core's files does not change another Core's count, path, identity, or data; no path is reused. | Integration-tested |
| **STO-05** | A failure in one Core (open failure, corrupt DB, invalid manifest) is reported per Core and does not prevent the Platform from starting or another Core from operating. Evidence: Platform status, per-Core status, B's successful operation while A is degraded. | Starting with a corrupt A must not abort Platform startup, must not globally STOP, must not delete A, and must not make B unreadable. | Integration-tested |

### 3.2 Filename and path collision

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **FIL-01** | Core storage paths are derived from the canonical Core UUID (or another documented non-user-controlled immutable key), never from display name or core number alone. Two Cores with the same display name produce different paths. Evidence: fixture with duplicate display names, distinct paths, distinct instance UUIDs. | Renaming a Core to another Core's display name does not move, overwrite, or merge either database. Display name `..\victim` or `Core B` cannot influence the path. | Integration-tested |
| **FIL-02** | UUID inputs are canonicalized to one lowercase hyphenated form before path/registry use. Uppercase, braced, URN, and simple forms of the same UUID resolve to the same identity/path or are rejected as duplicates. Evidence: input forms, canonical stored form, duplicate-registration denial. | Registering `A9A0...` after `a9a0...` must not create a second database or registry row. A nil UUID is rejected. | Unit-tested + Integration-tested |
| **FIL-03** | Collision-resistant path components handle Windows case-insensitivity and reserved names. Reserved device names (`CON`, `NUL`, `PRN`, `AUX`, `COM1`-`COM9`, `LPT1`-`LPT9`), trailing dots/spaces, control characters, NUL, overlong components, and Unicode normalization collisions are rejected before file creation. Evidence: rejection codes and no partial directory. | A name that normalizes to an existing name (for example `core` vs `core.` or NFC vs NFD) must not create a second directory or overwrite an existing one. | Unit-tested + Integration-tested |

### 3.3 Path traversal, junction/symlink, and root enforcement

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **TRAV-01** | All untrusted path inputs (registry `storage_relative_path`, CLI root, UUID-derived component, any import/source path) reject `..`, `/`, `\\`, `:`, NUL, control characters, absolute paths, UNC/device paths, and drive-relative paths. PATH components may not be empty or `.`. Evidence: error codes, no file created outside root, root hash/tree unchanged. | An outside-root sentinel file is created; traversal attempts must not read, write, rename, or delete it, and must not create a directory adjacent to it. | Unit-tested + Integration-tested |
| **TRAV-02** | The root, every intermediate directory, the database file, the manifest, and the owner proof are checked for symlink/junction/reparse-point replacement. A path validated earlier is not trusted for a later open/write without a no-follow or canonical-containment re-check. Evidence: fixture with a symlink/junction at root, intermediate component, and final file; all rejected; outside target untouched. | TOCTOU test: swap an intermediate directory or file to a junction/symlink between validation and open. The operation must fail closed or operate only on the original in-root object; it must not write outside the root. | Integration-tested + documented residual risk |
| **TRAV-03** | After canonicalization, every created/opened/deleted Core file resolves under the configured Core root. Registry-provided paths are resolved against the root and rechecked. Evidence: canonical parent paths and containment assertion in the receipt. | A registry path such as `cores/../../outside/core.db` or a path that resolves outside through a link is rejected before any open/write/delete. | Integration-tested |
| **ROOT-01** | A Core root must be absolute, createable/openable as a directory, and must not be a symlink or junction. The root is canonicalized and permissions are applied according to the storage policy. Evidence: path metadata, canonical path, error on relative root, error on file root. | A relative root, a root pointing to a regular file, and a root symlink/junction are all rejected; the parent directory is not polluted with a partially created Core tree. | Unit-tested + Integration-tested |
| **ROOT-02** | Per-Core databases and backups are kept outside a Git worktree, per SPEC-13. If an owner override is permitted, it is explicit, recorded, and not a default. Evidence: `git -C <root> rev-parse --show-toplevel` relationship and an override/denial record. | A root inside the Aegis worktree (for example `D:\Aegis\var\...`) is rejected or requires a recorded owner override; it must not silently become a production default. | Integration-tested |

### 3.4 Immutable UUID and mutable display name

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **ID-01** | Core UUID is immutable and is the primary identity key. Rename, re-registration, restart, and storage reopen leave the UUID, core number, and creation timestamp unchanged; only display name and its revision/updated timestamp may change. Evidence: before/after identity rows and registry rows. | A public API attempt to set/change `core_uuid` or `core_number` is rejected or not exposed; no database file is renamed to a different UUID; a second identity cannot be minted for the same Core. | Integration-tested |
| **ID-02** | Display name is mutable presentation metadata. Valid rename persists, increments the identity revision, and leaves immutable fields unchanged. Empty/whitespace-only/control-character/over-160-character names are rejected without mutation. Evidence: rename results, revision delta, validation errors, unchanged UUID/number/created_at. | Rename to `""`, `"   "`, a control character, or 161 characters is rejected; the original name and revision remain; no partial write. | Unit-tested + Integration-tested |
| **ID-03** | There is one documented source of truth for display name across the identity record, Core storage metadata, and Platform registry. Rename is atomic enough that a restart/status cannot show contradictory names. Evidence: source-of-truth document and a restart test. | A crash/fault during rename results in either the old consistent state or the new consistent state, never a mixed name across stores. A stale registry copy is reconciled or reported, not silently trusted. | Integration-tested |
| **ID-04** | Nil UUIDs and malformed UUIDs are rejected. Accepted UUIDs are stored in canonical form. Evidence: contract and storage validation results. | Nil UUID, malformed string, and non-canonical duplicate are rejected or canonicalized exactly once; no path or registry row is created for the malformed form. | Unit-tested |

### 3.5 Ownership substitution and foreign data

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **OWN-01** | Core ownership is cross-checked across the database identity row, database file name/root, integrity manifest `core_uuid`, and registry entry. Mismatch fails closed with `OwnershipMismatch`/`ForeignOwnership`. Evidence: deliberately mismatched fixtures and denial receipts. | Open Core A's DB with B's registry entry; replace A's manifest with B's; point B's registry path at A's DB. All are rejected; no cross-Core read/write; both DB hashes unchanged. | Integration-tested |
| **OWN-02** | Missing or ambiguous ownership metadata fails closed. The implementation never infers an owner from a filename, display name, directory order, or "only one Core present". Evidence: fixture with missing identity/ownership fields; denial and quarantine/report. | Deleting the ownership row or manifest `core_uuid` must not cause the DB to be adopted as another Core or as a fresh Core. | Integration-tested |
| **OWN-03** | A database copied from another Core or from legacy material is not accepted as a fresh Core. Evidence: copy fixture, `AlreadyExists`/`ForeignOwnership` error, original and copy hashes unchanged. | A copied DB with a different directory name or display name is rejected on init/open; no data is merged or imported. | Integration-tested |

### 3.6 Fresh initialization

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **INIT-F-01** | A fresh Core DB records a new `database_instance_uuid`, current schema revision, `initialization_mode = fresh_no_migration`, `source_database = none`, and creation timestamp. Historical stores exist and are empty. Evidence: identity/metadata row, manifest, per-store row counts (zero), integrity report. | A request that omits or changes the initialization mode/source must not silently become fresh; unsupported values are rejected. | Integration-tested |
| **INIT-F-02** | Fresh initialization rejects any source/history import and does not migrate legacy data. Evidence: source-path/non-empty fixture rejection; legacy DB hash and mtime unchanged. | Pointing fresh init at a legacy DB, a populated DB, or another Core's DB is rejected; no historical memory/conversation/relationship/emotion/wardrobe/activity rows are copied. | Integration-tested |
| **INIT-F-03** | Legacy/reference databases remain read-only and untouched. Evidence: SHA-256 and mtime before/after, no writable handle, no journal sidecars created beside legacy files. | A failed fresh init must not "clean up" or delete legacy material or its directory. | Integration-tested |
| **INIT-F-04** | Integrity is sealed before the Core becomes visible as registered/installed. Evidence: ordering log/checkpoint and registry visibility at each stage. | No window exists where the registry reports a registered Core whose DB or manifest is missing/invalid. | Integration-tested |

### 3.7 Repeated initialization

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **INIT-R-01** | Repeating init for the same Core is non-destructive and non-duplicating. Either it returns an explicit idempotent result or `AlreadyExists`; it does not create a new database instance, new registry row, new identity revision, or new owner proof. Evidence: before/after instance UUID, row counts, file hashes, returned status. | A second init with a different display name or UUID at the same path must not mutate the first Core, rename it, or overwrite its DB. | Integration-tested |
| **INIT-R-02** | Registering the same Core UUID under two roots is rejected or explicitly documented as unsupported; there must not be two simultaneously usable instances with the same UUID. Evidence: duplicate-root fixture and denial. | A copied/moved DB presented as a second instance is detected by database-instance UUID/ownership and rejected; no "last writer wins". | Integration-tested |
| **INIT-R-03** | Retry after a failed/interrupted init follows a documented rule: resume only from a validated checkpoint, otherwise fail closed and quarantine residue. Evidence: retry receipt, same or new instance UUID as documented, no duplicate registry entry. | Retry cannot silently overwrite a partially initialized DB or register a half-built Core. | Integration-tested |

### 3.8 Interrupted and partial initialization

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **INIT-P-01** | Fault injection after each stage (`validated_identity`, `storage_prepared`, `schema_applied`, `identity_written`, `integrity_sealed`, `registered`) leaves a checkpoint/report with the last durable stage and a failure code. The Core is not reported as fully registered unless the final stage completed. Evidence: per-stage receipts, checkpoint rows, report status, registry state. | A failure at each stage must not yield `Complete` or a registered Core; no stage may skip from e.g. `validated_identity` to `registered`. | Integration-tested |
| **INIT-P-02** | Crash windows are handled: DB without registry, registry without DB, manifest without DB, and DB without manifest are detected, reported, and not silently repaired. Residue is quarantined or left for an explicit retry. Evidence: orphan fixtures, status/reports, no data deletion. | An orphan DB must not be auto-adopted as a fresh Core; an orphan registry row must not become `active`; no partial Core is exposed as healthy. | Integration-tested |
| **INIT-P-03** | Partial temp files, `-wal`/`-shm` sidecars, half-written manifests, and staging directories are not accepted as valid state. Cleanup is explicit and does not delete pre-existing data. Evidence: file inventory before/after, quarantine receipt. | An otherwise valid-looking but incomplete manifest must be reported invalid, not `Verified`; a zero-byte temp file must not be treated as the DB. | Integration-tested |
| **INIT-P-04** | The multi-store init (Core DB, manifest, Platform registry) has a defined ordering and recovery. There is no cross-store transaction claim. Evidence: documented ordering + fault injection at each boundary. | A crash between DB commit and registry commit cannot produce an `active` entry; a crash between registry commit and manifest seal cannot produce a `Verified` entry. | Integration-tested |

### 3.9 Schema revision

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **SCH-01** | Each Core DB records the current schema revision, and the integrity manifest records the same revision. Evidence: `PRAGMA user_version` or the schema table, manifest field, equality assertion. | A DB revision that is newer or older than the supported revision is rejected with a schema/revision error; no write occurs and no automatic downgrade occurs. | Unit-tested + Integration-tested |
| **SCH-02** | Unknown schema names/versions are rejected; application markers are verified before domain writes. Evidence: wrong application id, wrong schema string, revision 0, and unsupported revision fixtures all fail closed. | A foreign SQLite file with a different application id/schema must not be initialized, repaired, or partially registered. | Integration-tested |
| **SCH-03** | Revisions are scoped per component (Platform, Safe, each Core), not coupled by one global constant that forces unrelated stores to migrate together. Evidence: bump Platform revision only; Core open still succeeds; Core revision unchanged. | A Platform revision change must not silently rewrite or reject an otherwise valid Core DB; a Core revision change must not require Safe/Platform data mutation. | Integration-tested |

### 3.10 Integrity: success, tamper, missing, invalid

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **INT-01** | Integrity success requires all of: manifest schema/algorithm valid; manifest `core_uuid` matches the DB/registry; `database_file` is a safe basename; SHA-256 is 64 lowercase hex; actual DB bytes/size match; SQLite integrity check passes; schema revision matches; ownership matches. Evidence: manifest and DB digests, SQLite result, report with `Verified`. | Any one failing condition must not produce `Verified`. A manifest with an uppercase or malformed digest is `InvalidManifest`, not `Verified`. | Integration-tested |
| **INT-02** | The integrity report names the expected and actual digest, expected and actual schema revision, status, and a content-free detail. Evidence: report JSON and receipt. | A report that says `Verified` without actual digest/schema/ownership checks is rejected as insufficient evidence. | Integration-tested |
| **INT-03** | Tampering with one byte of the DB while the manifest is unchanged yields `DigestMismatch`; no automatic repair, no silent regeneration, no overwrite. Evidence: before/after hashes, status, original file preserved. | Tampering must not be reported as `Verified`; the verifier must not "fix" the manifest to match the tampered DB. | Integration-tested |
| **INT-04** | Missing DB yields `DatabaseMissing`; missing manifest yields `MissingManifest`. Evidence: status and report; no files auto-created. | Missing artifacts must not cause an empty DB or empty manifest to be generated as if valid. | Integration-tested |
| **INT-05** | Invalid manifests are rejected: malformed JSON, wrong schema, wrong algorithm, uppercase/odd-length/non-hex digest, path separators or `..` in `database_file`, invalid/nil `core_uuid`, empty `created_at`, revision 0. Evidence: one receipt per malformed case with `InvalidManifest` or the documented specific error. | A manifest pointing at a file outside the Core directory (`../...`, `C:\...`, `\\server\...`) must not be followed. | Integration-tested |
| **INT-06** | Schema or ownership mismatch yields `SchemaMismatch` or `OwnershipMismatch`; SQLite corruption yields `SqliteCorrupt`. Evidence: separate fixtures and reports. | Corrupt/foreign DBs must not be reported `Verified` merely because a SHA-256 happens to match a stale manifest. | Integration-tested |
| **INT-07** | Digest establishes content integrity, not authenticity. The status/report wording must not claim the DB is "authentic", "trusted", "authorized", or "owner-approved". Evidence: documented status semantics and user-facing text. | If the DB and the manifest are both tampered consistently, the verifier must not claim authenticity; at most it reports consistency with the supplied manifest and the documented limitation. | Integration-tested + documentation review |

### 3.11 Database hashing versus WAL/journal state

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **WAL-01** | Integrity verification never hashes a stale main DB file while uncheckpointed WAL content exists. Before hashing, the implementation either closes/checkpoints the DB (and verifies no `-wal`/`-shm` remains) or returns a non-`Verified` result. Evidence: sidecar inventory, journal mode, checkpoint result, digest basis. | Write data in WAL mode, leave `-wal`/`-shm`, and hash the main file without checkpointing. The verifier must not report `Verified` against the pre-write manifest. | Integration-tested |
| **WAL-02** | Verification is not torn by a concurrent writer. The verifier uses an exclusive/read-consistent snapshot or returns `NotVerifiable`/`Busy`; it does not hash a file while another connection writes. Evidence: concurrency fixture and result. | A writer that commits during verification must not produce a digest that matches neither the before nor after state. | Integration-tested |
| **WAL-03** | The manifest/verification documentation defines the hash basis: exact file bytes, journal mode, whether sidecars are included, and the quiescence requirement. Evidence: documentation and manifest fields. | A future switch from `journal_mode=DELETE` to WAL must not silently change verification semantics; a versioned change is required. | Documentation + Integration-tested |

### 3.12 Deactivation preserving data

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **DEACT-01** | Deactivation changes lifecycle/registration status only. Core DB, integrity manifest, identity record, and Core-owned data remain present with unchanged hashes. Evidence: before/after hashes for DB and manifest, registry state, file inventory. | Deactivation must not delete, truncate, move, rename, or quarantine the Core DB or manifest; it must not clear the identity record. | Integration-tested |
| **DEACT-02** | Deactivation uses a lifecycle state and transition that are legal in the approved six-state contract (`design_only`, `approved`, `installed`, `active`, `suspended`, `retired`) or an explicitly versioned extension. Evidence: transition record and contract-version note. | `active -> retired` without owner approval, `design_only -> active`, `approved -> active`, and any transition for prohibited Cores #4/#5 are rejected. | Integration-tested |
| **DEACT-03** | A suspended Core can be reactivated without creating a new database instance or losing data; a retired Core is not silently reactivated. Evidence: suspend/resume test, instance UUID and data hashes preserved, retired denial. | Reactivation of a retired Core, or reactivation that creates a new DB instance UUID, is rejected or explicitly documented as a new restore flow. | Integration-tested |

### 3.13 Uninstall preserving data by default

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **UNINST-01** | Uninstall defaults to removing package/registration only; Core DB, manifest, assets, and backups remain. Evidence: before/after file inventory and hashes, uninstall receipt listing preserved paths. | Uninstall without an explicit purge request must fail if it would delete any Core data; no `remove_dir_all`/`remove_file` on the Core DB/manifest. | Integration-tested |
| **UNINST-02** | Explicit purge requires owner proof plus a separate destructive confirmation and produces a tombstone/deletion receipt; partial deletion is reported incomplete. Evidence: authorized purge receipt, content-free tombstone, backup policy reference. | Purge with a wrong/missing owner token is denied and deletes nothing. A wildcard or parent-directory delete is rejected. | Integration-tested |
| **UNINST-03** | After uninstall-preserve, status distinguishes "zero registered Cores" from "no data on disk". Evidence: status/report fields and retained-data listing. | Status must not imply the Core was deleted or that data is gone when it is preserved. | Integration-tested |

### 3.14 One-Core failure isolation and zero-Core startup

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **ISO-01** | With Cores A and B, corrupt A's DB/manifest, start Platform: A is reported degraded/failed, B remains readable/writable, Platform status is available. Evidence: per-Core status, B operation, A denial, no global STOP. | A's failure must not block B, delete A/B data, or auto-repair A; A's private content must not appear in B's errors/logs. | Integration-tested |
| **ISO-02** | No failure of one Core causes a cross-Core cascade, global STOP, or automatic quarantine deletion. Evidence: audit/receipt and file hashes. | A malicious/corrupt A manifest must not influence B's integrity result or registry path. | Integration-tested |
| **ZERO-01** | A fresh Platform starts with `zero_core = true`, `core_count = 0`, no implicit Core, no Core registry rows, and no Core database directories. Evidence: Platform status from API, CLI, and IPC; file inventory. | Startup must not auto-create a default Core, a placeholder Core #1, or a Core DB. | Integration-tested + Runtime-tested |
| **ZERO-02** | Registering a Core changes status to `zero_core = false`; uninstall-preserve returns to zero registered while retaining data, and status makes that distinction visible. Evidence: transition receipts and status snapshots. | The transition must not overwrite the zero-Core identity or delete the preserved Core data. | Integration-tested |

### 3.15 CLI behavior

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **CLI-01** | The CLI exposes the Batch 2 operations (init, status, Core create/list/show/rename, integrity verify, deactivate, uninstall) with documented JSON output, stable field names, and exit code 0 on success / non-zero on error. Evidence: command transcripts and JSON. | Missing arguments, unknown commands, invalid roots, and unsupported lifecycle actions return non-zero with a structured error; no panic/backtrace as normal output. | Runtime-tested |
| **CLI-02** | CLI path/root inputs are subject to the same root/traversal checks as the library. Evidence: relative/outside/link/traversal roots rejected; no file created outside. | A traversal display name or registry path supplied through the CLI must not create or delete files outside the root. | Runtime-tested |
| **CLI-03** | No secret leakage: `init` does not print the owner token unless an explicit development flag is used; logs/receipts do not contain the token, DPAPI blob, private Core content, or manifest secrets. Evidence: output/log scan and receipt scan. | `--print-owner-token` output must include a development-only warning; without the flag, stdout/stderr must not contain the token. | Runtime-tested |
| **CLI-04** | Destructive commands are not default behavior. `uninstall` preserves data; `purge` (if present) requires explicit owner proof and confirmation. Evidence: help text and command receipts. | A bare `uninstall` or a wrong-token purge must delete nothing; no recursive delete of a parent/shared directory. | Runtime-tested |
| **CLI-05** | Repeated status/list/verify commands are deterministic and idempotent; rename/list do not create duplicates. Evidence: repeated command outputs and row/file counts. | Repeating init/uninstall/rename must not duplicate registry rows or rotate identity/owner proof unexpectedly. | Runtime-tested |

### 3.16 IPC contract compatibility

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **IPC-01** | Batch 1 request/response schemas and protocol `1.0` remain compatible. Existing Batch 1 methods (`platform.status`, `platform.effect.ping`, `safe.stop.status`, `safe.stop.set`, `safe.stop.clear`) behave unchanged. Evidence: Batch 1 integration tests plus a compatibility round-trip. | An unmodified Batch 1 client must still parse responses and receive the same error codes for unauthorized, malformed, and unsupported requests. | Integration-tested |
| **IPC-02** | New Batch 2 methods are additive and registered in the contract. Breaking changes require a new schema/protocol version. Evidence: contract diff and method registry. | An unknown method returns `unsupported_method`; it must not be silently dispatched, interpreted as a file path, or fall through to an internal error. | Unit-tested + Integration-tested |
| **IPC-03** | Every Batch 2 IPC method is subject to owner-proof verification at the Safe boundary. Evidence: wrong/missing token denials for every new method. | No Batch 2 method may bypass authorization by using a different code path, a CLI shortcut, or an unauthenticated transport. | Integration-tested |
| **IPC-04** | Framing limits are enforced: frame > `MAX_FRAME_BYTES` yields `frame_too_large`; malformed JSON yields `invalid_json`; missing/invalid fields yield `invalid_request`; responses contain exactly one of `result`/`error`; `request_id` is echoed/validated. Evidence: contract tests. | An oversized or malformed frame must not be partially processed or cause a panic; a response with both/neither result and error is rejected. | Unit-tested + Integration-tested |
| **IPC-05** | IPC parameters cannot select arbitrary database/filesystem paths. The root is configured server-side; any path-like parameter is validated or rejected. Evidence: malicious parameter fixtures and denials. | A request containing `../../outside`, an absolute path, a UNC path, or another Core's UUID must not open/read/write that target. | Integration-tested |

### 3.17 Lifecycle and unauthorized transitions

| ID | Required behavior and exact evidence | Explicit negative test | Min class |
|---|---|---|---|
| **LIFE-01** | Only legal transitions are accepted: `design_only -> approved -> installed -> active`, `active <-> suspended`, and `any -> retired` only with owner approval. `installed` does not imply `active`. Evidence: transition table test for every pair and a transition log with from/to/generation/authority. | Skips (`design_only -> active`, `approved -> active`), backward transitions not explicitly allowed, and implicit promotion are rejected with no partial state change. | Integration-tested |
| **LIFE-02** | Prohibited Core numbers #4/#5 cannot be initialized, registered, transitioned, activated, or assigned a UUID. Evidence: per-number denial and audit. | A #4/#5 fixture is rejected at every entry point (library, CLI, IPC, registry); no database or registry row is created. | Integration-tested |
| **LIFE-03** | Lifecycle changes require the correct owner/Platform authority. No Core or model self-promotes. Stale generation/authority claims are rejected where generation is implemented. Evidence: wrong-token, stale-generation, and self-promotion denials. | An unauthenticated or stale request must not change state; a Core-originated "activate me" request must not be treated as authority. | Integration-tested |
| **LIFE-04** | Registry uniqueness is defined and enforced for Core UUID, core number, storage path, and (if policy requires) display name. Evidence: duplicate insert attempts and constraint/denial results. | Two registry rows cannot point at the same DB path or claim the same Core number under the chosen policy; duplicate UUID cannot exist. | Integration-tested |
| **LIFE-05** | Retired is terminal for normal operation. Uninstall/removal does not silently delete data and does not create a new active instance. Evidence: retired/uninstall receipts and status. | A retired Core cannot be reactivated without an explicit owner-approved recovery/restore path; an uninstalled Core's data remains. | Integration-tested |
| **LIFE-06** | The lifecycle vocabulary is explicitly reconciled: the approved six-state contract versus the master specification's broader generic lifecycle list (which includes `Failed`, `Deactivated`, `Uninstalled`). Any added state is versioned in contracts and tested; any mapping (for example deactivation -> `suspended`) is documented. Evidence: contract version note and mapping table. | An undocumented `deactivated`/`uninstalled` string must not be accepted or persisted; a state not in the contract must not be silently coerced to another state. | Documentation + Unit-tested |
| **LIFE-07** | `active` is not entered without the required health/authority/STOP conditions. Evidence: activation test with STOP active and with missing authority. | Activation while STOP is active, while integrity is not Verified, or without owner/Platform authority is rejected. | Integration-tested |

---

## 4. Top acceptance checks the orchestrator must run after integration

Run in this order; stop on the first failure and retain the receipt.

1. **Repository hygiene and build**
   - `git -C D:\Aegis status --short` confirms only intended files changed.
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
   - Require all four to pass in the pinned Rust 1.88.0 toolchain.

2. **Contract compatibility gate**
   - Confirm `aegis-contracts` still compiles with existing Batch 1 crates.
   - Run Batch 1 IPC round-trip and Platform tests unchanged.
   - Reject any breaking change to `IPC_REQUEST_SCHEMA`, `IPC_RESPONSE_SCHEMA`, or protocol `1.0` without a new version.

3. **Storage isolation gate (STO-01 through STO-05)**
   - Create at least three synthetic Cores under one root.
   - Assert distinct paths, instance UUIDs, application markers/table ownership, and no cross-Core reads.
   - Corrupt one Core and assert the other two still work.

4. **Path and link attack gate (TRAV-01 through TRAV-03, ROOT-01/02, FIL-01 through FIL-03)**
   - Run traversal inputs through library, CLI, and IPC.
   - Exercise symlink/junction at root, intermediate directory, DB file, and manifest.
   - Exercise a validation-to-open swap (TOCTOU) and assert no outside-file write.
   - Assert the root is outside the Git worktree or has a recorded owner override.

5. **Identity and ownership gate (ID-01 through ID-04, OWN-01 through OWN-03)**
   - Rename and restart: UUID/core number/created_at unchanged.
   - Substitute A's DB/manifest/registry path into B: all denied with ownership errors.
   - Confirm missing ownership metadata fails closed.

6. **Initialization gate (INIT-F-01 through F-04, INIT-R-01 through R-03, INIT-P-01 through P-04)**
   - Create a fresh Core and verify instance UUID, mode, source none, empty stores.
   - Repeat init and verify no duplication.
   - Inject a fault after every init stage and verify checkpoint/report/registry consistency.
   - Verify no orphan DB is auto-adopted.

7. **Schema gate (SCH-01 through SCH-03)**
   - Open with older/newer/unknown revisions and assert rejection without writes.
   - Verify per-component revision independence.

8. **Integrity gate (INT-01 through INT-07, WAL-01 through WAL-03)**
   - Verify a clean DB.
   - Flip one byte and verify `DigestMismatch`.
   - Remove DB/manifest and verify `DatabaseMissing`/`MissingManifest`.
   - Feed malformed manifests and verify `InvalidManifest`.
   - Create `-wal`/`-shm` content and verify no false `Verified`.
   - Confirm the report does not claim authenticity.

9. **Lifecycle gate (DEACT-01 through DEACT-03, UNINST-01 through UNINST-03, LIFE-01 through LIFE-07)**
   - Verify legal/illegal transition matrix.
   - Verify deactivation and uninstall-preserve leave DB/manifest hashes unchanged.
   - Verify #4/#5 denial and owner-auth requirement.
   - Resolve/document the six-state versus broader lifecycle vocabulary before claiming this gate.

10. **Runtime gate (CLI-01 through CLI-05, IPC-01 through IPC-05, ZERO-01 through ZERO-02, ISO-01 through ISO-02)**
    - Run the built CLI against a disposable absolute root outside the worktree.
    - Run zero-Core status, register, rename, verify, deactivate, uninstall-preserve.
    - Run the IPC server/client sequence and adversarial frames.
    - Scan output/logs for owner tokens and private content.

11. **Evidence audit**
    - Every row has a receipt with a command, environment, timestamp, result, and hashes.
    - Negative tests show both denial and no side effects.
    - No row claims Live-verified or Owner-accepted.
    - A-04, B-01, B-06 remain flagged provisional.

12. **Independent re-review**
    - Re-review the actual source diff, not agent summaries.
    - Produce a new disposition. This document is not an approval.

---

## 5. Exit criteria

Batch 2 may be considered **integration-ready** only when:

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all pass in the pinned toolchain.
- All rows intended for Wave 1b have `pass` receipts at the declared class, including a new cross-crate Batch 2 integration suite (storage + identity + integrity + registry + Platform status + CLI/IPC).
- The critical gaps in Section 1.2 are fixed and covered by tests: one `core_identity` schema, one registry source of truth, a stored `database_instance_uuid`, atomic rename/reseal, end-to-end init, a real deactivation/uninstall-preserve flow, hardened paths/permissions, and ownership on every integrity verification.
- No unresolved high-severity QA finding remains without an owner decision.
- The six-state lifecycle vocabulary, database-instance UUID, ownership metadata, schema revision, and integrity-manifest semantics are frozen in contracts or explicitly versioned.
- A-04, B-01, and B-06 remain unresolved/provisional in all output.
- No adult capability, production, Live-verified, or Owner-accepted claim exists.
