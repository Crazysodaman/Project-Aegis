# Batch 5 / Agent 10 — Core Lifecycle, Identity & Database Integrity

Baseline: git commit `2bd2513`, working tree with pre-existing uncommitted work
preserved. Scope limited to: `aegis-core-identity`, `aegis-core-storage`,
`aegis-core-integrity`, `aegis-core-registry`, `aegis-core-install`,
`aegis-core-runtime`, `aegis-roster`, `aegis-core-profiles`.

No source files were modified. No new dependencies were added. No live database
under `D:\Aegis\var` was touched. All new tests use `tempfile` roots and
`PlatformService::initialize` on disposable directories.

## Status

PASS. New integration tests added and green; clippy is clean with `-D warnings`.
No production defect was found, so no additive `src` change was required.

## Files changed (exact)

New test files only:

- `crates/aegis-core-install/tests/lifecycle_integrity.rs` (5 tests)
- `crates/aegis-core-identity/tests/recovery_disk.rs` (3 tests)
- `crates/aegis-roster/tests/roster_policy.rs` (4 tests)
- `crates/aegis-core-profiles/tests/profile_policy.rs` (3 tests)

New document:

- `docs/implementation/BATCH5_AGENT10_CORE_INTEGRITY.md` (this file)

Nothing else was created, edited, or deleted by this agent.

## Verification commands and counts

```
cargo test -p aegis-core-identity -p aegis-core-storage -p aegis-core-integrity \
  -p aegis-core-registry -p aegis-core-install -p aegis-core-runtime \
  -p aegis-roster -p aegis-core-profiles
```

Result: **119 passed, 0 failed** across the eight packages (15 of these are new).

Per test binary:

| Package | Test binary | Tests |
|---|---|---|
| aegis-core-identity | `identity_initialization` | 19 |
| aegis-core-identity | `recovery_disk` (new) | 3 |
| aegis-core-install | `core_installation` | 12 |
| aegis-core-install | `lifecycle_integrity` (new) | 5 |
| aegis-core-integrity | unit tests | 16 |
| aegis-core-profiles | unit tests | 3 |
| aegis-core-profiles | `profiles` | 6 |
| aegis-core-profiles | `profile_policy` (new) | 3 |
| aegis-core-registry | `registry` | 19 |
| aegis-core-runtime | `runtime` | 9 |
| aegis-core-storage | `core_storage` | 13 |
| aegis-roster | unit tests | 7 |
| aegis-roster | `roster_policy` (new) | 4 |

(Plus the empty unit/doc-test binaries for each package, all 0 tests.)

```
cargo clippy -p aegis-core-identity -p aegis-core-storage -p aegis-core-integrity \
  -p aegis-core-registry -p aegis-core-install -p aegis-core-runtime \
  -p aegis-roster -p aegis-core-profiles --all-targets -- -D warnings
```

Result: **exit code 0**, no warnings.

## What the new tests prove

### `crates/aegis-core-install/tests/lifecycle_integrity.rs`

1. `install_verify_and_inspect_report_the_same_identity` — install → verify
   integrity (`Verified`) → inspect returns the same identity, storage metadata,
   and a registered `installed` entry.
2. `rename_preserves_uuid_core_number_and_database_instance` — rename changes
   only the display name, increments `identity_revision`, reseals/re-verifies the
   manifest, and keeps `core_uuid`, `core_number`, `created_at`, and
   `database_instance_uuid` unchanged.
3. `deactivate_then_reactivate_preserves_database_instance_and_data` —
   `installed → suspended → active` preserves the database instance UUID, the
   Core identity, the database and manifest bytes, and unrelated files inside the
   Core directory; integrity still verifies.
4. `uninstall_preserve_retains_core_directory_and_data` — `uninstall_preserve`
   moves the registry entry to `retired` while leaving the Core directory,
   `core.db`, `core.integrity.json`, retained files, and hashes untouched;
   integrity still verifies.
5. `two_cores_have_distinct_databases_and_instance_uuids` — two Cores get
   separate database paths/directories and distinct `database_instance_uuid`
   values, neither equal to its own Core UUID.

### `crates/aegis-core-identity/tests/recovery_disk.rs`

Uses the existing checkpoint API (`write_checkpoint` / `read_checkpoint` /
`recover_decision` / `InitializationStateMachine::start`) across a real
write/read cycle:

1. An interrupted run (`SchemaApplied`) is `Interrupted`, requires explicit
   cleanup, and `start` returns `InterruptedInitialization`.
2. A failed run (`Failed` + failure code) is `Failed`, permits an explicit
   restart at `ValidatedIdentity`/`InProgress` with no failure code.
3. A completed run is `Complete` and refuses reinitialization with
   `AlreadyInitialized`.

This complements the existing install-level tests
(`interrupted_checkpoint_blocks_silent_recovery`,
`failed_checkpoint_with_existing_database_can_be_recovered`).

### `crates/aegis-roster/tests/roster_policy.rs`

- #4 and #5 are `Prohibited`, not identity-defined, not implementation
  authorized, not installable, and are rejected by `resolve_display_name` with
  `ProhibitedCore` for both `None` and supplied names.
- #7 is implementable but not identity-defined, has no default name, and
  `resolve_display_name` requires an owner-supplied name (empty/whitespace is
  rejected; supplied names are trimmed).
- Catalog is exactly `[1,2,3,4,5,6,7]`; #1 preserves the legacy Sofía identity;
  #2/#3/#6 names are as approved.
- Out-of-range numbers are rejected.

### `crates/aegis-core-profiles/tests/profile_policy.rs`

- #4/#5 profiles are design-only/prohibited, have no display name, identity
  attributes, personality, appearance, or voice, produce no personality, and are
  rejected for display-name resolution.
- #7 profile is a blank foundation owned by the owner: no display name, gender,
  age, species, personality, appearance, or voice, produces no personality, and
  requires an owner-supplied name.
- The catalog validates and the approved names/sources for #1/#2/#3/#6 match the
  roster.

## Audit

### Package

| Area | Finding |
|---|---|
| Platform package | `aegis-installer` owns platform install/upgrade/uninstall and records a versioned installation record in `platform.db`. It never creates, runs, or deletes Cores. |
| Core "package" | A Core is composed of the Core directory (`cores/<uuid>/core.db`), the integrity manifest (`core.integrity.json`), and the init marker/checkpoint (`init/<uuid>.initialization.marker`). There is no signed Core package artifact; integrity is content integrity, not authenticity. |
| Versioning | Core schema revision is recorded in `aegis_schema` and in the manifest; manifests do not carry a package version/publisher. |

### Install / initialize

`aegis-core-install::CoreInstaller::initialize` requires owner authorization
(`SafeStore::verify_owner`), validates the identity request, checks the
checkpoint, refuses silent adoption of an existing unregistered database, then
runs a deterministic state machine:

`ValidatedIdentity → StoragePrepared → SchemaApplied → IdentityWritten →
IntegritySealed → Registered`.

Each stage is checkpointed atomically. Failures are written as a `Failed`
checkpoint. A completed checkpoint returns `AlreadyInitialized`; an interrupted
checkpoint returns `Interrupted` and is never silently resumed.

### Activate / deactivate

| Operation | Registry rule | Data effect |
|---|---|---|
| `activate` | `installed`/`suspended → active` | none |
| `deactivate` | `installed`/`active → suspended` | none (DB/manifest untouched) |
| `reactivate` | `suspended → active` only | none |
| runtime `start` | activates `installed`, reactivates `suspended` | spawns `aegis-core-host` |
| runtime `stop` | `active → suspended` | terminates host, preserves DB/manifest |

All installer lifecycle methods require the owner token. All transitions append
to `core_lifecycle_change`; repeating a transition is rejected as
`IllegalTransition`.

### Uninstall

`CoreRegistry::uninstall_preserve` moves `installed`/`active`/`suspended` to
`retired` and only updates the Platform registry row plus append-only history.
Destructive deletion is intentionally not implemented; the registry test asserts
the source contains no `remove_file`, `remove_dir`, or `remove_dir_all`. The new
install-level test additionally proves the Core directory, database, manifest,
and retained files survive retirement with unchanged hashes.

### Registry / identity

- Identity: `CoreIdentity` keeps `core_uuid`, `core_number`, and `created_at`
  immutable; only `display_name` changes, incrementing `identity_revision`.
  UUIDs are canonicalized; nil and malformed UUIDs are rejected. Numbers 4/5 are
  prohibited; numbers outside `{1,2,3,6,7}` are unapproved.
- Registry: `core_registry` has unique `core_uuid`, `core_number`, and
  `storage_relative_path`; `lifecycle_state` is constrained; `core_number` is
  constrained to `1,2,3,6,7`; display-name updates are validated and mutate only
  presentation metadata. Reserved `platform.db`/`safe.db` paths are rejected.
  Legacy `platform_core` reconciliation is one-way and only runs when the new
  registry is empty.

### Database ownership

- Each Core has its own `core.db` with application marker `AEGC`
  (`CORE_APPLICATION_ID = 0x4145_4743`), distinct from Platform and Safe markers.
- `core_identity` is a singleton row with a unique `database_instance_uuid` that
  must not equal the Core UUID.
- Opens are read-only ownership-checked before a writable open; foreign UUID,
  foreign marker, substituted file, and tampered identity row are rejected.
- Interrupted/partial directories and databases without an identity row are
  rejected (no silent adoption or reuse).
- Paths are constrained to the canonical `<root>/cores` tree; traversal,
  absolute paths, reserved Platform/Safe files, and redirected components are
  rejected.

### Memory isolation (adjacent, not modified)

`aegis-core-sdk` stores per-Core memory in `cores/<uuid>/memory.db` (marker
`AEGM`) with an immutable ownership row protected by append-only/immutable
triggers. `CoreMemoryStore::open` first verifies the Core's `core.db` snapshot
and then the memory ownership row. Existing SDK tests cover cross-Core isolation,
foreign-ownership rejection, and path containment away from Platform/Safe. These
were audited, not duplicated here, because `aegis-core-sdk` is outside this
agent's file ownership.

### Conversation persistence (adjacent, not modified)

`aegis-conversation` stores per-Core conversations in
`cores/<uuid>/conversation.db` (marker `AEGX`), bound to the Core UUID and
database instance UUID. Existing tests cover turns persisting across reopen and
per-Core database isolation/foreign-ownership rejection. Audited only; not
modified or duplicated.

### Core Link

There is no component literally named "Core Link" in the repository. The closest
equivalents are:

- IPC between Platform/CLI and the service (`aegis-ipc` request/response).
- The supervised per-Core link to `aegis-core-host`: a child process spawned by
  `CoreRuntime` with `--root` and `--core-id`, exchanging liveness through
  `run/<uuid>.heartbeat` and `run/<uuid>.stop`.
- Registry/runtime reconciliation: runtime start activates a Core and runtime
  stop returns an active Core to suspended.

Gaps for this link are listed below (no per-Core authenticated channel, and
runtime/registry divergence is not event-reconciled).

## Verified roster expectations

| # | Expected | Verified by |
|---|---|---|
| 1 | Sofía Ada Lyra, preserved | roster name + `preserve_legacy_identity`; profile `PreservedLegacy` + voice reference; install/inspect tests use this identity |
| 2 | Mack Fuller | roster/profile name match; install test installs #2 |
| 3 | Nova Arden | roster/profile name match |
| 4, 5 | Design-only twins, prohibited | roster `Prohibited` + not installable; profile `DesignOnly`/`Prohibited` + empty identity; contract rejects prohibited numbers; registry CHECK excludes them |
| 6 | Vera / S-039, display "Vera" | roster/profile name match; profile `ApprovedExternalSpecification` |
| 7 | Blank/unwritten, owner-supplied name | roster/profile have no name or identity fields; require an owner name; installable only after a name is supplied |

## Defects found / fixed

None. All new tests passed against the baseline implementation on the first run,
so no production code was changed.

## Remaining gaps / limitations

1. **Integrity is not authenticity.** The manifest is an unkeyed SHA-256 of the
   database plus metadata; a party who can rewrite both database and manifest can
   produce a self-consistent pair. There is no signature/publisher chain.
2. **No signed/versioned Core package.** Cores are composed in place from
   database + manifest + marker; there is no distributable package artifact with
   version pinning or package-level trust.
3. **No destructive uninstall/purge.** `uninstall_preserve` is the only uninstall
   path; a confirmed, owner-authorized purge with a receipt is not implemented.
4. **Core Link hardening.** There is no dedicated per-Core authenticated link;
   the runtime link is a child process plus heartbeat/stop files. Prior QA notes
   residual risks: heartbeat temp-file naming/containment (`QA3-07`) and
   runtime/registry divergence without a reconciliation event (`QA3-27`).
5. **TOCTOU/junction residuals.** Earlier batches documented residual
   symlink/junction swap windows and Windows reparse-point coverage gaps
   (`QA-05`, `QA-30`, `QA-32`). This batch did not change those paths.
6. **Blank Core #7 identity workflow.** Installation is possible once the owner
   supplies a display name, but no separate workflow persists additional identity
   attributes; the catalog correctly refuses to invent them.
7. **Memory and conversation isolation** are covered by their own crates'
   tests; this batch audited them but did not add cross-crate tests because those
   crates are outside this agent's ownership.
8. Concurrent uncommitted changes from other agents (e.g. embodiment, fleet,
   service, other Batch 5 documents) are unrelated to this report and were not
   modified.

## Next steps

- Add an authenticated Core Link (per-Core token or signed descriptor) and
  reconcile runtime/registry divergence with an append-only event.
- Add package signing / publisher trust above the existing content-integrity
  manifests.
- Define and test an owner-confirmed, backup-gated purge path distinct from
  `uninstall_preserve`.
- Close the Windows junction/reparse TOCTOU residuals with open-once or
  no-follow handles and swap tests.
