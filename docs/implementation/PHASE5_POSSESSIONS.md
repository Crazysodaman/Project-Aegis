# Phase 5 — Per-Core Possessions

Status: implemented in `aegis-core-sdk`. The possessions store is owned by one
Core, bound to that Core's identity, and stores real Core-authored items,
outfits, documents, world objects, and an append-only event ledger.

## Files

- `crates/aegis-core-sdk/src/possessions.rs` — schema, store, API, validation.
- `crates/aegis-core-sdk/src/lib.rs` — module, re-exports, `CoreSdkContext`
  field and `CoreSdkContext::possessions()` accessor.
- `crates/aegis-core-sdk/tests/possessions.rs` — 11 integration tests.
- `docs/implementation/PHASE5_POSSESSIONS.md` — this document.

No other crate, CLI, workspace, or storage file is changed by this milestone.

## Database identity and isolation

- File: `<core>/possessions.db`.
- SQLite application marker: `AEGO` (`0x4145_474F`).
- Component: `core_possessions` in the shared `aegis_schema` table.
- Ownership table `possessions_ownership` has exactly one immutable row bound
  to **both** `core_uuid` and `database_instance_uuid`. The instance UUID is
  read only from the Core's `core.db` snapshot when the store opens.
- `possessions_ownership` rejects `UPDATE` and `DELETE` with triggers.
- The store opens only the verified Core directory, rejects symlinks and
  foreign paths, and rejects a copied database whose ownership row does not
  match the target Core or database instance.
- Cross-Core reads are not exposed: every query is filtered by the store's
  `core_uuid`. Opening another Core's store returns that Core's own database,
  which contains none of the first Core's rows.

## Schema

### `item_instances`

Indexed columns: `instance_id`, `core_uuid`, `holder_core_uuid`,
`definition_id`, `name`, `category`, `state`, `visibility`,
`condition_percent`, `location`, `created_at`, `updated_at`, `revision`.
The complete `PossessionItem` (`ItemInstance` plus Core-authored name,
description, category, and properties) is stored as `payload_json`.

Indexes: `(core_uuid, category)`, `(core_uuid, state)`,
`(core_uuid, updated_at)`.

### `outfits`

`outfit_id`, `core_uuid`, `name`, `item_instance_ids_json`, `visibility`,
`archived`, `created_at`, `updated_at`, `revision`.
Archived outfits are excluded from `list_outfits` but remain in the database.
Index: `(core_uuid, archived, updated_at)`.

### `documents`

`document_id`, `core_uuid`, `title`, `body`, `format`, `visibility`,
`created_at`, `updated_at`, `revision`. The body is persisted verbatim,
including line breaks and tabs. Index: `(core_uuid, updated_at)`.

### `world_objects`

`object_id`, `core_uuid`, `definition_id`, `owner_core_uuid`,
`holder_core_uuid`, `location`, `state`, `affordances_json`, `visibility`,
`created_at`, `updated_at`, `revision`. Index: `(core_uuid, updated_at)`.

### `possession_events`

`sequence` (autoincrement), `core_uuid`, `entity_kind`, `entity_id`, `action`,
`detail_json`, `visibility`, `created_at`. Index: `(core_uuid, sequence)`.
`UPDATE` and `DELETE` are rejected by triggers; events are append-only.

### `possessions_schema`

Schema marker `aegis.core.possessions.v1`, revision `1`, with an immutable
single row.

## API

`PossessionsStore::open(root, core_uuid)` verifies the Core and opens or creates
the database.

Items:

- `create_instance(&NewInstance) -> PossessionItem`
  - Optional catalog `definition_id` (stored only for a later catalog link).
  - Core-authored `name`, `description`, `category`, `properties`.
  - Owner is this Core; default state is `stored`; default visibility is
    `private`.
- `list_inventory(limit) -> Vec<PossessionItem>`
- `get_instance(instance_id) -> Option<PossessionItem>`
- `set_state(instance_id, state) -> PossessionItem`
  - Changes use state only (`stored`, `worn`, `held`, Core-defined states).
  - Does **not** change owner or holder.
- `update_instance(instance_id, &InstanceUpdate) -> PossessionItem`
  - Owned items only. Updates name, description, category, visibility,
    properties, condition, and location. Owner/holder/definition stay fixed.

Outfits:

- `create_outfit(&NewOutfit) -> Outfit`
- `list_outfits(limit) -> Vec<Outfit>`
- `update_outfit(outfit_id, name, item_ids, visibility) -> Outfit`
- `archive_outfit(outfit_id) -> ()`

Every outfit item must be owned by this Core or currently held by this Core.
Unknown items are rejected; items owned by another Core are rejected with
`SdkError::ForeignCore`. Item identifiers must be unique and bounded.

Documents:

- `create_document(title, body, format, visibility) -> Document`
- `get_document(document_id) -> Option<Document>`
- `update_document(document_id, title, body, format, visibility) -> Document`
- `list_documents(limit) -> Vec<Document>`

Every update increments the document revision. No hidden memory claim is used:
the returned body is the persisted body.

World objects:

- `create_object(&NewObject) -> WorldObject`
- `update_object(object_id, &ObjectUpdate) -> WorldObject`
- `list_objects(limit) -> Vec<WorldObject>`

Events:

- `record_event(&NewPossessionEvent) -> PossessionEvent`
- `list_events(limit) -> Vec<PossessionEvent>`

Events are append-only. Built-in operations also append events (`created`,
`state_changed`, `updated`, `archived`, transfer offers).

Transfer offers:

- `create_transfer_offer(instance_id, to_core_uuid) -> TransferOffer`

This records an offer event with status `offered` and does not transfer,
copy, move, or re-own anything. The item remains owned by the offering Core.
Delivery requires the Phase 5 transfer subsystem.

## Privacy and ownership semantics

- `Visibility` is stored on every item, outfit, document, object, and event.
  New records default to `Private`.
- Since each database belongs to exactly one Core, private data cannot be read
  through another Core's store. Foreign keys and ownership rows prevent opening
  a copied database as another Core.
- Ownership (`owner_core_uuid`) and wearing/use (`state`, `holder_core_uuid`)
  are separate. `set_state("worn")` leaves the owner and holder unchanged.
- Outfit membership uses the same ownership/held check; mere knowledge of a
  foreign identifier is not enough to add it to an outfit.

## Bounds

- Names: `MAX_NAME_CHARS`; descriptions/text: `MAX_TEXT_CHARS`.
- Document body: `MAX_DOCUMENT_BODY_CHARS`; titles: `MAX_NAME_CHARS`.
- Item/outfit/object lists: at most `MAX_LIST_ITEMS` entries per record.
- JSON properties: `MAX_PROPERTIES_BYTES` (64 KiB).
- Event detail: `MAX_EVENT_DETAIL_BYTES` (16 KiB).
- Listings: `MAX_POSSESSION_LIST` records; event listings `MAX_EVENT_LIST`.
- All JSON payloads are parsed and validated on read; corrupt rows surface as
  `SdkError::CorruptData` rather than silently returning malformed data.

## What is real in this milestone

- A real per-Core `possessions.db` with application marker, schema marker,
  immutable dual-bound ownership, and append-only event triggers.
- Core-authored item instances without requiring a catalog definition, with
  persisted name, description, category, and properties.
- Separate owner and use state; worn/held state changes do not alter owner.
- Real persisted outfit membership with ownership/held validation.
- Real persisted document bodies with revisions.
- Real persisted world object location, state, and affordances.
- Reopen persistence for every record kind.
- Privacy by default and rejection of foreign/copied databases.
- Transfer offers recorded as status events without faking delivery.

## Next steps (not in this milestone)

- Phase 5 transfer subsystem: actual delivery, acceptance, ownership handoff,
  and holder updates across Core boundaries.
- Trading, gifting, lending, and return workflows with explicit consent.
- Catalog link: resolve `definition_id` against the shared catalog, enforce
  definition versions, and support catalog-derived defaults.
- Item definitions authored by Cores and published with `PublicCatalog`
  visibility.
- UI/TUI inventory, outfit, document editor, and object interaction surfaces.
- Module/runtime integration: actions that read/write possessions through this
  SDK under existing authorization rules.
- Schema migration framework if the possessions schema revision advances.

## Verification

```text
cargo test -p aegis-core-sdk
cargo clippy -p aegis-core-sdk --all-targets -- -D warnings
```

The new possessions integration test file contains 11 tests covering identity
binding, create/own/wear separation, outfit foreign-item rejection, document
body and revision persistence, object state, reopen persistence, cross-Core
copy rejection, privacy, transfer offers, append-only events, and bounds.
