# Wave 2b: Platform-Mediated Item Transfers

This milestone adds a real, Platform-mediated `TransferStore` to
`aegis-platform`. The Platform records transfer offers, enforces the transfer
lifecycle, and coordinates delivery between two existing Core
`PossessionsStore` instances. The only platform database change is additive:
new `item_transfers` and `item_transfer_events` tables in `platform.db`.

No existing Platform store, Core SDK store, CLI, workspace, or Core database is
modified. All mutations require Safe owner proof.

## Schema (`platform.db`)

### `item_transfers`

| Column | Meaning |
| --- | --- |
| `transfer_id` | Primary key, a new UUID per offer. |
| `from_core_uuid` | Canonical sender Core UUID. |
| `to_core_uuid` | Canonical recipient Core UUID, distinct from the sender. |
| `instance_id` | Sender's item instance identifier at offer time. |
| `transfer_type` | `gift`, `loan`, `trade`, `swap`, or `return`. |
| `status` | `pending`, `accepted`, `rejected`, `cancelled`, or `failed`. |
| `note` | Bounded, caller-supplied note. |
| `created_at` / `updated_at` | RFC 3339 timestamps. |
| `delivered_instance_id` | Recipient's new instance UUID, set only on acceptance. |
| `revision` | Monotonic row revision starting at 1. |

Indexes cover status/update order and each Core side of the transfer.

### `item_transfer_events`

| Column | Meaning |
| --- | --- |
| `sequence` | Auto-increment primary key. |
| `transfer_id` | Owning transfer. |
| `event_type` | `offered`, `accepted`, `rejected`, `cancelled`, or `failed`. |
| `status` | Lifecycle status at the time of the event. |
| `detail_json` | Event-specific JSON detail (identifiers, reason, etc.). |
| `created_at` | RFC 3339 timestamp. |

`BEFORE UPDATE` and `BEFORE DELETE` triggers abort any attempt to mutate or
delete an event, making the lifecycle ledger append-only.

## API

All mutation methods take a Safe owner token and return `TransferError`:

- `offer(owner_token, from_core, to_core, instance_id, transfer_type, note)` —
  validates distinct canonical UUIDs, opens the sender store, and calls
  `export_for_transfer` to prove current ownership. Foreign or missing items are
  rejected before anything is written. On success, inserts a `pending` row and
  an `offered` event in one `platform.db` transaction.
- `get(transfer_id)` and `list(limit)` — bounded reads.
- `events(transfer_id, limit)` — append-order lifecycle events.
- `accept(owner_token, transfer_id)` — only pending transfers are eligible.
- `reject(owner_token, transfer_id)` / `cancel(owner_token, transfer_id)` — only
  pending transfers are eligible; they append an event and perform no delivery.

Limits: `TRANSFER_DEFAULT_LIMIT = 50`, `TRANSFER_MAX_LIMIT = 200`,
`TRANSFER_MAX_EVENTS = 500`, `TRANSFER_MAX_NOTE_CHARS = 512`.

## Lifecycle

```
offer -> pending
pending -> accepted   (successful delivery)
pending -> rejected   (owner)
pending -> cancelled  (owner)
pending -> failed     (stale item, import/removal error, platform update error)
```

`accepted`, `rejected`, `cancelled`, and `failed` are terminal; a second
`accept`/`reject`/`cancel` returns `NotPending`.

## Delivery and compensation

`accept` performs these steps in order:

1. Re-export the item from the sender store. Failure (missing item or changed
   ownership) marks the transfer `failed`, appends a `failed` event, and returns
   the error. Nothing is delivered.
2. Import the exported payload into the recipient as a new instance.
   Failure marks the transfer `failed` and appends an event.
3. Remove the source instance from the sender. If removal fails, the imported
   instance is rolled back at the recipient, the transfer is marked `failed`,
   and the error is returned.
4. Only after delivery succeeds, update the Platform row to `accepted` with the
   delivered instance ID and append an `accepted` event.

The recipient item carries the source provenance plus a `transferred_in` entry
whose note is the `transfer_id`, so accepts are traceable end to end.

Because `platform.db` and the two `possessions.db` files are separate SQLite
databases, this is **not** a distributed transaction. If the final Platform
update fails after delivery, `accept` attempts best-effort compensation: it
re-exports the delivered item from the recipient, imports it back into the
sender, removes it from the recipient, and marks the record `failed`. It never
reports `accepted` in that case.

## Limitations

- Delivery is a two-database compensation protocol, not an atomic distributed
  transaction. A crash or repeated compensation failure can leave a partial
  state that requires manual reconciliation.
- The sender row is deleted on success (the append-only possession ledger keeps
  its history); compensation may restore it under a new instance UUID.
- Transfers move item instances only. Outfits, documents, and world objects are
  out of scope.
- The Platform is the authority for the transfer record, but it does not add
  per-Core authentication beyond Safe owner proof.
- Events are append-only and are never compacted or deleted.
