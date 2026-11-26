# Phase 5 - Platform Shared Item Catalog

**Date:** 2026-11-26
**Baseline:** clean HEAD `6d84eb4` (existing uncommitted lived-environment
contracts preserved).
**Scope:** Platform-owned shared item catalog persisted in `platform.db`,
built on `aegis_contracts::living::ItemDefinition`.

---

## 1. Status

Implemented. No resets, rebases, commits, new dependencies, or workspace-wide
gates were run. Only the assigned files were touched.

### Files

| File | Change |
|---|---|
| `crates/aegis-platform/src/catalog.rs` | New `CatalogStore`, `CatalogFilter`, `CatalogActor`, `CatalogError`, catalog tables and events. |
| `crates/aegis-platform/src/lib.rs` | Added `mod catalog;` and public re-exports only. |
| `crates/aegis-platform/tests/catalog.rs` | New catalog integration tests (9 tests). |
| `docs/implementation/PHASE5_CATALOG.md` | This document. |

`world.rs`, `weather.rs`, the CLI, the workspace manifest, and all other crates
were not modified.

---

## 2. Storage schema (`platform.db`)

The catalog is additive. Existing Platform tables and `aegis_schema` revision
handling are untouched.

### `catalog_definitions`

| Column | Type | Notes |
|---|---|---|
| `definition_id` | `TEXT PRIMARY KEY` | Immutable catalog identity. Duplicate IDs are rejected. |
| `definition_json` | `TEXT NOT NULL` | Full validated `ItemDefinition` JSON. |
| `creator_core_uuid` | `TEXT` | Caller-supplied; canonical UUID or `NULL`. Never invented by Platform. |
| `category` | `TEXT NOT NULL` | Indexed copy of the definition category. |
| `visibility` | `TEXT NOT NULL` | Indexed copy of the definition visibility. |
| `created_at` | `TEXT NOT NULL` | Store-owned on create; preserved on update. |
| `updated_at` | `TEXT NOT NULL` | Store-owned on create/update. |
| `revision` | `INTEGER NOT NULL` | `>= 1`; store-owned. Updates are `revision + 1`. |
| `archived` | `INTEGER NOT NULL` | `0`/`1`, defaults to `0`. |
| `archived_at` | `TEXT` | Set when archived. |
| `archived_by` | `TEXT` | Actor label for the archive event. |

Indexes cover category, creator, visibility, and the deterministic
`(updated_at, definition_id)` ordering.

### `catalog_events` (append-only)

| Column | Type | Notes |
|---|---|---|
| `sequence` | `INTEGER PRIMARY KEY AUTOINCREMENT` | Monotonic event order. |
| `event_type` | `TEXT NOT NULL` | `catalog.created`, `catalog.updated`, `catalog.archived`. |
| `definition_id` | `TEXT NOT NULL` | Affected definition. |
| `revision` | `INTEGER NOT NULL` | Definition revision at event time. |
| `actor` | `TEXT NOT NULL` | `owner` or an explicit `CatalogActor` label. |
| `detail_json` | `TEXT NOT NULL` | Bounded event detail (IDs, revision, category, visibility, creator, tags). |
| `created_at` | `TEXT NOT NULL` | Event timestamp. |

Two `BEFORE UPDATE` / `BEFORE DELETE` triggers abort attempts to mutate or erase
history, so `catalog_events` is append-only at the database boundary. Archiving
never deletes the definition row or its events.

---

## 3. API

```rust
let catalog = CatalogStore::new(paths); // StoragePaths
catalog.initialize()?;

let created = catalog.create(&owner_token, definition)?;
let fetched = catalog.get("def-id")?;              // Option<ItemDefinition>, active only
let page    = catalog.list(&CatalogFilter::default())?;
let found   = catalog.search("linen")?;
let updated = catalog.update_own(&owner_token, "def-id", replacement)?;
catalog.archive(&owner_token, "def-id")?;          // alias: delete
```

- `create` / `create_as`: requires Safe owner proof, calls
  `ItemDefinition::validate`, validates creator UUID, rejects duplicate IDs,
  sets `revision = 1` and store-owned timestamps, and appends `catalog.created`.
- `get`: returns an active definition or `None`; archived entries stay in the
  database but are hidden.
- `list(&CatalogFilter)`: filters by category, visibility, creator, and tag;
  hides archived entries unless `include_archived` is set; default limit 50,
  hard maximum 200; zero or oversized limits are rejected; ordering is
  `updated_at DESC, definition_id ASC`.
- `search(text)`: bounded non-empty text (max 256 chars) matched against the
  full definition JSON (name, description, tags, category, creator), same
  bounded limit and deterministic ordering. `LIKE` wildcards are escaped.
- `update_own` / `update_own_as`: requires owner proof, validates the
  replacement, preserves the stored creator, `definition_id`, and `created_at`,
  forces `revision + 1` and a fresh `updated_at`, and appends `catalog.updated`.
- `archive` / `delete` / `archive_as` / `delete_as`: requires owner proof,
  marks the row archived, and appends `catalog.archived`. Re-archiving is
  idempotent and does not add a duplicate event.

Creator IDs are accepted only as canonical lowercase hyphenated UUIDs. A
non-canonical UUID or any attempt to change the creator on update is rejected.

---

## 4. Authority model

- The Platform owns the catalog database and all authority. Cores do not open
  `platform.db` directly.
- Every mutation calls `SafeStore::verify_owner`. A wrong token returns
  `CatalogError::Unauthorized`; an uninitialized owner proof returns the
  underlying Safe error. Read-only operations do not require owner proof.
- `CatalogActor` is an explicit actor label paired with an authorized Safe
  owner token. The token grants authority; the label (`owner`, `core:...`, or a
  caller-defined label) is recorded in `catalog_events` for audit. The token is
  redacted from `Debug` output.
- Visibility is part of the definition. Creating or changing a public/shared
  definition therefore requires owner proof like any other mutation.
- The Platform never synthesizes `creator_core_uuid`; it persists exactly what
  the caller supplied (or `NULL`).

### Intentionally not done yet

- **Core write authorization:** Cores have no dedicated Platform write
  credential. A Core cannot mutate the catalog unless a trusted Platform caller
  supplies it an owner-authorized `CatalogActor`; that is a Platform delegation,
  not a Core identity check.
- **Transfers and ownership assignment:** no instance ownership, trades,
  gifting, or transfer workflow exists in this phase.
- **UI / IPC:** no catalog IPC methods or GUI/TUI surfaces were added; the API
  is a library surface only.
- **Retention/compaction:** events are append-only and never pruned. No export,
  migration, or archival-policy tooling exists yet.
- **Full-text search:** search uses bounded SQLite `LIKE` matching over stored
  JSON, not an index-backed search engine.

---

## 5. Verification

Commands run (scoped, not workspace-wide):

```text
cargo test -p aegis-platform
cargo clippy -p aegis-platform --all-targets -- -D warnings
```

### Test evidence

New `tests/catalog.rs` (9 tests, all passing):

| Test | Coverage |
|---|---|
| `create_get_list_and_search_by_category_tag_and_creator` | create, get, category/tag/creator/visibility list, text search, bounds. |
| `list_limits_are_bounded_and_ordered` | default/max limits, rejected zero/oversized limits, deterministic ordering. |
| `duplicate_definition_ids_are_rejected` | duplicate ID error and single stored row. |
| `invalid_definitions_and_creators_are_rejected` | `ItemDefinition::validate` failure, malformed and non-canonical creator UUIDs. |
| `catalog_survives_reopen_with_events` | persistence across a new `CatalogStore` instance and event persistence. |
| `events_are_append_only_and_archive_preserves_history` | created/updated/archived event sequence, idempotent archive, triggers reject UPDATE/DELETE. |
| `owner_proof_is_required_for_mutations_including_visibility` | wrong-token rejection for create/update/public visibility/archive, owner publish works. |
| `explicit_catalog_actor_with_owner_token_is_recorded` | `CatalogActor` authorization and actor labels in events. |
| `catalogs_do_not_cross_platform_boundaries` | separate temp roots stay isolated; catalog lives in `platform.db`, not `safe.db`. |

Package result: **51 passed, 0 failed** (`aegis-platform` unit + integration
tests). Clippy: clean with `-D warnings`.

---

## 6. Next steps

1. Add Platform-mediated catalog IPC methods (`catalog.create/list/get/update/archive`) with owner proof.
2. Introduce a real Core-scoped write credential or delegation model so Cores
   can author catalog entries without borrowing the owner token.
3. Add item-instance ownership/provenance and transfer workflows on top of the
   shared catalog.
4. Add catalog-aware UI/TUI surfaces and export/migration tooling once the
   authority model is finalized.
