# Batch 5 — Agent 08: Artifact Library & Data Management

Status: implemented and verified (`aegis-embodiment` only).

## Scope

This batch extends the per-Core embodiment artifact library with additive,
backward-compatible APIs for metadata search, integrity auditing, and recovery
manifest generation. No existing behavior, database schema, or file layout was
changed. Per-Core isolation, ownership binding, SHA-256 verification, soft delete,
and export semantics are preserved.

All assets remain under `<root>/cores/<core_uuid>/assets/<asset_id>/<safe-name>`
and are bound to the owning Core's immutable Core UUID and database instance UUID.
No cross-Core reads were introduced.

## Added APIs

### 1. Metadata indexing/search

- `AssetFilter { kind, name_contains, tags, media_type, include_deleted, limit }`
- `EmbodimentStore::list_filtered(&AssetFilter) -> Vec<Asset>`
- `EmbodimentStore::search(&AssetFilter) -> Vec<Asset>` (alias)
- `AssetFilter::matches(&Asset) -> bool`

Filter semantics:

- `kind`: exact asset-kind match.
- `name_contains`: case-insensitive substring match on the asset name.
- `tags`: every requested tag must be present; comparison is case-insensitive.
- `media_type`: case-insensitive exact match.
- `include_deleted`: include soft-deleted records (default `false`).
- `limit`: result bound, clamped to `MAX_ASSET_SEARCH_LIMIT` (1000). `None`
  defaults to the same bound; `Some(0)` returns no results.

Ordering is deterministic: `created_at ASC, asset_id ASC`, matching the existing
`list_assets` ordering.

### 2. Integrity / recovery audit

- `EmbodimentStore::audit() -> AssetAudit`
- `EmbodimentStore::verify_all() -> AssetAudit` (alias)
- `AssetAudit { schema, core_uuid, audited_at, entries, verified, missing, corrupt }`
- `AssetAuditEntry { asset_id, kind, name, sha256, bytes, state, relative_path, outcome }`
- `AuditOutcome { Verified, Missing, Corrupt }`
- `AssetAudit::is_clean()`

The audit re-hashes every live asset, verifies path containment, rejects
redirected/symlinked files, and classifies each entry as `verified`, `missing`,
or `corrupt`. It never deletes, rewrites, or otherwise mutates assets or records.
Soft-deleted assets are excluded from the audit; their files are retained for
owner recovery.

### 3. Recovery manifest / export helper

- `EmbodimentStore::recovery_manifest() -> AssetManifest`
- `AssetManifest { schema, core_uuid, generated_at, entries }`
- `AssetManifestEntry { asset_id, kind, name, media_type, sha256, bytes, state, relative_path }`
- `AssetState { Live, Deleted }`

The manifest lists live and soft-deleted assets with the asset id, kind, name,
media type, SHA-256, size, state, and path relative to the owning Core's root
directory (e.g. `assets/<asset_id>/<safe-name>`). It is serializable for backup
tooling.

## Per-Core isolation

All new APIs operate through `EmbodimentStore`, which is opened for exactly one
Core and enforces ownership on open. Search, audit, and manifest queries are
scoped by `core_uuid` and read files only from that Core's directory. Tests
confirm that a second Core sees no assets, no audit entries, and an empty
manifest.

## Verification

- `cargo test -p aegis-embodiment` — 11 passed (7 existing + 4 new), 0 failed.
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings` — clean.

New tests (`crates/aegis-embodiment/tests/library.rs`):

- `search_filters_by_kind_name_tags_and_media_type`
- `audit_reports_corrupt_and_missing_without_mutating`
- `recovery_manifest_lists_live_and_deleted_assets`
- `search_audit_and_manifest_are_core_scoped`

## Limitations

- **Cross-host synchronization is NOT implemented.** There is no network
  replication, peer discovery, conflict resolution, or remote backup transport.
  Every store is local to its Core directory on the current host.
- Search filtering for tags/name/media type is applied in memory after a bounded
  Core-scoped query; it is not a separate persistent search index.
- The audit reads whole files into memory to re-hash them, so auditing very
  large libraries is I/O and memory proportional to asset size.
- SHA-256 verifies content integrity on read/audit; it does not establish
  publisher authenticity or provenance.
- Soft delete retains asset files; no garbage collection or orphan cleanup is
  performed (by design, for recovery).
- The manifest describes metadata and relative paths only; it does not copy
  artifact bytes or appearance configuration. Combine it with the existing
  `export()` for full appearance data.

## Required steps for cross-host synchronization (future work)

1. Define a transport and trust model (mutual authentication, per-Core identity,
   authorization) before any asset bytes leave a host.
2. Add a replication log or change-feed keyed by Core UUID and revision so
   imports, soft deletes, and appearance updates can be replayed idempotently.
3. Extend the recovery manifest with content hashes, revision numbers, and
   tombstones (already partially present) and add a verified transfer format.
4. Implement conflict resolution for concurrent edits and deletes (last-writer
   wins is insufficient for auditability).
5. Validate ownership and re-hash every transferred asset on the receiver before
   committing it to the destination Core's directory.
6. Add failure recovery, resumable transfer, and rate/space limits.
7. Test multi-host scenarios, including interrupted transfers and hostile peers,
   before enabling replication.

## Files changed

- `crates/aegis-embodiment/src/asset.rs`
- `crates/aegis-embodiment/src/store.rs`
- `crates/aegis-embodiment/src/lib.rs`
- `crates/aegis-embodiment/tests/library.rs` (new)
- `docs/implementation/BATCH5_AGENT08_ARTIFACT_LIBRARY.md` (new)
