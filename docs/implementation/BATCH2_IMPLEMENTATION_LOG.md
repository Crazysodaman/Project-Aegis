# Batch 2 — Wave 1b Implementation Log

**Date:** 2026-11-22  
**Target:** `D:\Aegis` only  
**Reference repository:** `D:\SofiaDev\SofiaAdaLyra` (strictly read-only)  
**Scope:** Wave 1b per-Core storage, identity, initialization, integrity, registry, CLI. No Wave 1c/1d or Wave 2/3 work.

## 1. Sub-agent orchestration

**Five sub-agents were actually launched concurrently:** four implementation agents plus one independent QA/security reviewer.

| Agent | Responsibility | Outcome |
|---|---|---|
| 1 | `aegis-core-storage`: per-Core DB composition, paths, ownership, isolation | Implemented; 12 tests |
| 2 | `aegis-core-identity`: immutable UUID, mutable name, init state machine/checkpoints | Implemented; 18 tests |
| 3 | `aegis-core-integrity`: SHA-256 manifests, schema/SQLite/ownership verification | Implemented; 13 tests |
| 4 | `aegis-core-registry`: Platform roster, lifecycle history, deactivation | Implemented; 9 tests |
| 5 | QA/security/architecture review and acceptance criteria | Independent review; reproduced a UUID bug; final verdict approve-with-conditions after remediation |

The primary orchestrator owned shared contracts, Cargo workspace integration, the new `aegis-core-install` composition layer, CLI integration, cross-crate tests, and conflict resolution.

## 2. Contracts added/changed

Added to `aegis-contracts`:

- `core.rs` — `CoreIdentity`, `CoreStorageMetadata`, UUID/name/number validation, canonical UUID helper.
- `initialization.rs` — request/report/checkpoint contracts, stages, statuses, failure codes, `fresh_no_migration`.
- `integrity.rs` — manifest/report contracts, integrity status, SHA-256 validation.
- `roster.rs` — registration/registry/lifecycle-change contracts and safe relative-path validation.

Changed:

- `CoreStorageMetadata` now includes `database_instance_uuid`.
- `CoreIdentity::new` and `with_display_name` canonicalize UUIDs to lowercase hyphenated form.
- Existing Batch 1 IPC/status contracts remain compatible.

## 3. New crates

- `crates/aegis-core-identity`
- `crates/aegis-core-storage`
- `crates/aegis-core-integrity`
- `crates/aegis-core-registry`
- `crates/aegis-core-install` (orchestrator)

`aegis-cli` was extended. Root `Cargo.toml` workspace membership and shared dependencies were updated. No reference-repository file was modified.

## 4. Database architecture

Per installation root:

```text
platform.db
safe.db
owner.proof
cores/<core-uuid>/core.db
cores/<core-uuid>/core.integrity.json
init/<core-uuid>.initialization.marker
```

Core DB application marker: `AEGC` (`0x4145_4743`). Platform marker: `AEGP`. Safe marker: `AEGS`.

`core_identity` contains:

- `schema`
- `core_uuid` (immutable)
- `database_instance_uuid` (new per fresh database)
- `core_number`
- `display_name` (mutable)
- `created_at`
- `identity_revision`
- `initialization_mode` (`fresh_no_migration`)
- `schema_revision`

Platform DB now contains `core_registry` and append-only `core_lifecycle_change`. Platform status counts `core_registry` entries when present.

## 5. CLI commands

```text
aegis core list --root <path>
aegis core init <core-id> --root <path> --number <n> --name <name>
aegis core inspect <core-id> --root <path>
aegis core verify <core-id> --root <path>
aegis core rename <core-id> --root <path> --name <name>
aegis core deactivate <core-id> --root <path> --reason <text>
```

`init`, `rename`, and `deactivate` require a valid Safe owner proof. All commands fail nonzero on identity/storage/integrity/authorization errors.

## 6. Integration issues found and fixed

Agent 5 independently reproduced and flagged:

1. **Schema mismatch:** Core storage’s `core_identity` lacked the `schema` column that integrity verification expects. Fixed by adding the column and aligning reads.
2. **Missing database instance UUID:** added `database_instance_uuid` to Core storage, contracts, and metadata.
3. **Registry/Platform mismatch:** Batch 1 `platform_core` was still counted after new `core_registry` registration. Platform `core_count` now prefers `core_registry`.
4. **Marker made Core directory non-empty:** initialization checkpoint originally lived inside `cores/<uuid>/`, causing `PartialState`. Moved to `init/<uuid>.initialization.marker`.
5. **Ownership skipped when identity table absent:** integrity now reports `OwnershipMismatch` instead of silently skipping.
6. **Display-name rename invalidated integrity:** orchestration now renames the DB, reseals the manifest, verifies, then updates the registry.
7. **No owner authorization for Core install/lifecycle:** `CoreInstaller` now verifies Safe owner proof for init/rename/deactivate.
8. **Uppercase UUID left partial residue:** reproduced by Agent 5. Fixed by canonicalizing UUIDs in identity construction and rename; added contract and installer regression tests. CLI smoke now succeeds with uppercase input and stores lowercase canonical identity.

## 7. Verification results

- `cargo fmt --all -- --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — **103 passed, 0 failed**.
- CLI build — pass.
- Multi-Core runtime smoke — passed:
  - Platform init.
  - Core A #1 and Core B #2 independent initialization.
  - Separate DBs and distinct database-instance UUIDs.
  - `core list`, `core inspect`, `core verify`.
  - Duplicate init rejected with exit 1.
  - Rename preserves UUID and reseals integrity.
  - Deactivate preserves DB/manifest hashes and reports `suspended`.
  - Tampered Core A fails `DigestMismatch`; Core B remains verified.
  - Platform status shows `core_count=2`, `zero_core=false`.
  - IPC status remains valid.
- Uppercase-UUID CLI smoke — pass after remediation; canonical lowercase stored.

Classification:

- **Designed:** yes.
- **Coded:** all five Wave 1b layers and CLI.
- **Built:** workspace and CLI built.
- **Unit-tested:** contracts, storage, identity, integrity.
- **Integration-tested:** storage isolation, initialization, integrity, registry, install orchestration, CLI/IPC.
- **Runtime-tested locally:** CLI multi-Core smoke, tamper/re-verify, deactivate, uppercase canonicalization.
- **Live-verified:** not claimed.
- **Owner-accepted:** not claimed.

## 8. Known limitations

- No production deployment, service installation, or live verification.
- No Linux build/runtime; Unix code paths are cfg-gated but untested here.
- No encryption at rest; OS permissions only.
- No WAL/sidecar/concurrent integrity test.
- TOCTOU between ownership check and writable open is mitigated by re-validation, not eliminated.
- Checkpoint replacement is not crash-atomic on Windows; it fails closed.
- Uninstall-preserving-data and reactivation workflows are not implemented.
- Database-instance UUID is not yet in the integrity manifest or registry.
- Public `CoreIdentity` fields/serde can bypass `new`; normal installer path validates.
- `platform_core` and `core_registry` coexist; migration rule deferred.
- **A-04, B-01, B-06 remain provisional.**
- Adult capability remains fail-closed; no Core cognition/memory/chat/voice/avatar/TUI added.
- No legacy database, asset, service, task, or reference-repository file was modified.
- No commit or push.

## 9. Agent 5 verdict

**Approve with conditions for Wave 1b documentation completion.** Not approved for production, live-verified, or owner-accepted use. The reproduced UUID high-severity issue was independently confirmed resolved for the normal CLI/installer path; residual risks are documented in `BATCH2_QA_REVIEW.md`.

## 10. Next boundary

Wave 1c is not started. It requires new owner authorization.
