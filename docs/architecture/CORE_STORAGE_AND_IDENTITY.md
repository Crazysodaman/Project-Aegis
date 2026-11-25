# Core Storage and Identity — Batch 2 / Wave 1b

**Status:** Implemented and tested locally; not production-deployed, live-verified, or owner-accepted.  
**Governing specifications:** AEGIS-MAB-003 v0.3.0; SPEC-03; SPEC-12; SPEC-13; locked decisions A-01–D-10.  
**Scope:** Wave 1b only. No memory, chat, cognition, relationships, consent system, voice, avatar, TUI, or adult capability.

## 1. Architecture

Wave 1b adds five implementation layers beneath the existing Batch 1 Platform/Safe foundation:

```text
aegis-core-identity   immutable UUID, mutable display name, init state machine/checkpoints
aegis-core-storage    per-Core SQLite path, schema, ownership, isolation
aegis-core-integrity  SHA-256 manifest, schema revision, SQLite/ownership verification
aegis-core-registry   Platform-side roster, lifecycle history, deactivation
aegis-core-install    orchestration of the four above + owner authorization
```

The existing layers remain authoritative:

- `aegis-contracts` — versioned shared contracts.
- `aegis-storage` — storage roots, app markers, schema revisions.
- `aegis-safe` — owner proof, STOP, audit.
- `aegis-platform` — installation identity, zero-Core status.
- `aegis-ipc` — local JSON-RPC transport.
- `aegis-cli` — local command surface.

No Core identity or private data is added to generic Platform code. Platform remains Core-neutral and operates with zero registered Cores.

## 2. Storage layout

Per installation root (configurable; local default under `D:\Aegis\var`):

```text
<root>/
  platform.db                         # Platform registry/config only
  safe.db                             # owner proof records, STOP, security audit
  owner.proof                         # DPAPI-protected (Windows) / 0600 Unix dev file
  cores/
    <core-uuid>/
      core.db                         # AEGC SQLite application marker
      core.integrity.json             # SHA-256 manifest
  init/
    <core-uuid>.initialization.marker # atomic-ish init checkpoint
```

The initialization marker deliberately lives outside `cores/<uuid>/`. An interrupted initialization must not make the Core database directory look like a non-empty partial database.

Core filenames use the canonical Core UUID. Display names never select a path. Platform DB and Safe DB remain physically separate from each Core DB.

## 3. Database ownership

| Store | Owner | Contents |
|---|---|---|
| `platform.db` | Platform | installation identity, `core_registry`, `core_lifecycle_change`, future Platform domains |
| `safe.db` | Safe | owner proof metadata, STOP, approvals, append-only security audit |
| `cores/<uuid>/core.db` | One Core | `core_identity`, schema marker, future Core-owned domains |
| `cores/<uuid>/core.integrity.json` | One Core | content digest, byte length, schema revision, identity digest |
| `init/<uuid>.initialization.marker` | Initialization | stage/failure checkpoint only |

Core DB schema currently contains:

```sql
aegis_schema(component TEXT PRIMARY KEY, revision INTEGER, created_at TEXT)
core_identity(
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  schema TEXT NOT NULL,
  core_uuid TEXT NOT NULL,
  database_instance_uuid TEXT NOT NULL UNIQUE,
  core_number INTEGER NOT NULL,
  display_name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  identity_revision INTEGER NOT NULL,
  initialization_mode TEXT NOT NULL,
  schema_revision INTEGER NOT NULL
)
```

`database_instance_uuid` is new on fresh creation and distinct from `core_uuid`. It supports SPEC-13’s database-instance identity requirement.

## 4. Identity model

- **Core UUID** is the immutable identity and path key.
- **Core number** is roster policy (approved `1,2,3,6,7`; prohibited `4,5`).
- **Display name** is mutable presentation metadata. Renaming preserves UUID, number, creation timestamp, and database; it increments `identity_revision`.
- **Database instance UUID** identifies one database instance; it is not the Core identity.
- UUID input is canonicalized to lowercase hyphenated form by `CoreIdentity::new` and `with_display_name`.
- Ownership is checked read-only before any writable open, then re-checked after the writable open. A foreign or tampered identity row fails closed.
- Cores #4 and #5 are rejected by contract policy.
- Core #7 has no invented identity; initialization requires an owner-supplied UUID and display name.

Public structs still expose fields directly and can be deserialized; callers that manually construct or deserialize `CoreIdentity` must validate before use. The normal installer path canonicalizes and validates.

## 5. Initialization process

`CoreInstaller::initialize(request, owner_token)` performs:

1. Verify Safe owner proof.
2. Validate request and canonicalize UUID.
3. Read/validate any initialization checkpoint.
4. Reject complete, interrupted, already-registered, or silently adopted unregistered DBs.
5. Create/reopen the per-Core DB under the approved `cores` root.
6. Apply schema and write immutable identity + mutable display name + `database_instance_uuid`.
7. Record `fresh_no_migration` and schema revision.
8. Verify SQLite integrity.
9. Seal `core.integrity.json` over exact database bytes.
10. Verify digest, schema revision, SQLite integrity, identity digest, and stored ownership.
11. Register the Core in `platform.db` as `installed`.
12. Advance the checkpoint to `registered`.

Failure writes a `failed` checkpoint. A failed checkpoint with an existing same-Core database may be retried; interception at a non-terminal stage requires explicit cleanup and is not silently resumed.

## 6. Integrity model

`core.integrity.json` records:

- Core UUID.
- Database file name (`core.db` only; no directory).
- Exact file SHA-256.
- Exact byte length.
- Schema revision.
- Canonical identity SHA-256.
- Algorithm (`sha256`).
- Creation timestamp.

Verification checks manifest presence/validity, Core UUID match, database presence, `PRAGMA integrity_check`, file digest/byte length, schema revision, manifest identity digest, and stored `core_identity` ownership. A missing identity row is an ownership failure.

**Integrity is not authenticity.** A digest proves that bytes match the manifest. It does not prove publisher identity, package trust, installation authorization, or resistance to an attacker with write access to both DB and manifest.

Display-name rename reseals the manifest after the authorized mutation. Any other database write that is not followed by an authorized reseal causes `DigestMismatch`.

## 7. Registry and lifecycle

`platform.db` contains:

- `core_registry` — one row per registered Core, unique UUID/number/storage path.
- `core_lifecycle_change` — append-only transition history.

Initial state after initialization is `installed`. `deactivate` allows `installed` or `active` → `suspended`; database and manifest are untouched. Destructive uninstall/deletion and automatic reactivation are not implemented in Wave 1b. Registry APIs support display-name synchronization but the orchestrator performs DB rename + reseal + registry update together.

Platform `core_count` reads `core_registry` when present; otherwise it retains the Batch 1 `platform_core` fallback. Zero registered Cores remains valid (`zero_core=true`).

## 8. Failure handling

- Invalid/prohibited UUID or Core number: rejected before storage.
- Foreign/redirected root, traversal, symlink/reparse component: rejected.
- Existing unregistered DB without checkpoint: refused (`ExistingUnregisteredDatabase`).
- Interrupted checkpoint: refused (`Interrupted`), explicit cleanup required.
- Foreign database ownership/marker: rejected.
- Missing/tampered manifest or DB: integrity failure; no silent repair.
- Integrity mismatch on one Core does not affect other Cores.
- Registration failure leaves a failed checkpoint; a retry with the same identity can recover.
- Owner proof failure prevents init/rename/deactivate.
- A Core failure does not prevent Platform startup.

## 9. CLI behavior

```text
aegis core list --root <path>
aegis core init <core-id> --root <path> --number <n> --name <name>
aegis core inspect <core-id> --root <path>
aegis core verify <core-id> --root <path>
aegis core rename <core-id> --root <path> --name <name>
aegis core deactivate <core-id> --root <path> --reason <text>
```

All commands return nonzero on failure. `init`, `rename`, and `deactivate` require a valid Safe owner proof; the CLI obtains the local development token from the existing Safe proof file.

## 10. Remaining limitations

- Windows DPAPI is a development owner proof; Unix uses an explicit 0600 dev fallback.
- Encryption at rest remains deferred (Q5-A).
- No Linux build/runtime was performed; Unix `cfg` paths are implemented but untested here.
- No WAL/sidecar/concurrent-write integrity test.
- TOCTOU between read-only ownership check and writable open is mitigated by re-checking after open, not eliminated.
- Checkpoint replacement is not crash-atomic on Windows; a failed replacement fails closed and may require cleanup.
- Core/sidecar permissions rely on inherited ACLs on Windows; Unix DB uses 0600 but sidecar hardening is not comprehensive.
- Database instance UUID is recorded in Core storage metadata but not yet in the integrity manifest or registry.
- Uninstall-preserving-data and reactivation workflows are not implemented.
- `platform_core` and `core_registry` coexist; a formal migration/deprecation rule is deferred.
- A-04 adult eligibility, B-01 final backup/host placement, and B-06 backup retention/RPO/RTO remain provisional.
- No production cutover, live verification, or owner acceptance is claimed.
