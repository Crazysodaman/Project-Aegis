# Wave 2a: Consent Registry and Possession Transfers

This milestone adds two independent, Core-bound surfaces to `aegis-core-sdk`:

1. A per-Core consent registry (`consent.db`) with append-only events.
2. Additive transfer payload APIs on `PossessionsStore`.

No existing store, contract, CLI, workspace, or platform behaviour is changed.

## Consent registry

### Storage

Each Core owns `<core>/consent.db` with SQLite application marker `AEGQ`
(`0x4145_4751`) and schema marker `aegis.core.consent.v1`.

| Table | Purpose |
| --- | --- |
| `consent_ownership` | Exactly one row binding `core_uuid` and `database_instance_uuid`. Immutable via triggers. |
| `consent_schema` | Schema marker and revision. |
| `consent_grants` | Current grant state, including `grant_id`, `scope`, `capability`, `participants_json`, `context`, `granted_at`, `expires_at`, `revoked_at`, and `revision`. |
| `consent_events` | Append-only ledger with `granted`, `revoked`, and `expired` actions. |

The database is opened only through `ConsentStore::open(root, core_uuid)`. The
Core database is read-only and supplies the immutable Core UUID and database
instance UUID. A copied `consent.db` from another Core is rejected with
`SdkError::OwnershipMismatch`. The store never writes to `core.db`, `memory.db`,
`personal.db`, or `possessions.db`.

### API

- `ConsentStore::open(root, core_uuid)`
- `grant(&ConsentGrant)` — validates, rejects duplicates/foreign or already
  revoked grants, and appends a `granted` event.
- `get(grant_id)`
- `list(limit)`
- `revoke(grant_id)` — sets `revoked_at` immediately, appends a `revoked`
  event, and advances the stored revision. It is idempotent.
- `active(scope?, capability?, participant?, now)`
- `check(scope, capability, participant, now) -> bool`
- `list_events(limit)`

Revocation and expiry take effect immediately for `check` and `active`.
Consent is never inferred from trust, ownership, personality, or memory.
Observing an expired grant records a single `expired` event in the ledger.

`CoreSdkContext::open` now opens the same consent store and exposes it via
`CoreSdkContext::consent()`.

## Possession transfers

`PossessionsStore` gains three additive operations; existing methods and
behaviour are unchanged.

- `export_for_transfer(instance_id)` — returns the owned `PossessionItem` with
  metadata and provenance. Foreign rows are rejected with
  `SdkError::OwnershipMismatch`.
- `import_transferred_item(source, from_core_uuid, transfer_id)` — creates a new
  instance with a fresh UUID, `owner_core_uuid = this Core`,
  `holder_core_uuid = None`, `state = "stored"`, copied metadata and provenance,
  plus a `transferred_in` provenance entry whose note is the transfer ID. The
  event ledger receives a `transferred_in` event.
- `remove_transferred_item(instance_id, transfer_id)` — deletes the owned source
  row after a successful transfer and appends a `transferred_out` event.
- `rollback_imported_item(instance_id, transfer_id)` — deletes an imported row
  when sender removal fails and appends a `transfer_rolled_back` event. The row
  must carry the matching `transferred_in` provenance entry.

Ownership is never mutated in place. A transfer is: export at the sender,
import at the receiver, then remove at the sender; rollback is used at the
receiver if sender removal fails. Deleted rows remain represented in the
append-only possession event ledger.

## Limitations

- Transfer is an explicit two-phase protocol driven by callers; there is no
  network transport or automatic coordination between Cores.
- Consent records are descriptive storage. Enforcement remains outside this SDK.
- Expiry events are recorded lazily when `active` or `check` observes an expired
  grant.
- Scope and participant matching are exact; no wildcards or hierarchy are
  implied.
- The transfer APIs move only item instances, not outfits, documents, or world
  objects.
