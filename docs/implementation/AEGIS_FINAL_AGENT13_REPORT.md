# Aegis FINAL - Agent 13: Social, Groups & Relationships

**Baseline HEAD:** `2bd2513`
**Scope:** `crates/aegis-social/**`, `docs/implementation/AEGIS_FINAL_AGENT13_REPORT.md`
**Constraints honored:** `D:\SofiaDev\SofiaAdaLyra` untouched; no commit/reset/
clean/deploy; no new dependencies; no workspace/CLI/other-crate edits; no
invented trust, relationship, or awareness state.

## Status

Complete. Additive only; existing public methods and behavior are preserved.

## Files changed

- `crates/aegis-social/src/relationships.rs`
  - `RelationshipEvent` now carries the `revision` it was recorded at.
  - `RelationshipStore::record_event(target_kind, target_id, event_type, detail)`
    appends one explicit caller-supplied event in a transaction and advances the
    relationship revision by one.
  - Created / updated / deleted relationship events now persist their revision.
  - `relationship_events` gained a `revision` column, with an in-place
    `PRAGMA table_info` migration (`ALTER TABLE ... ADD COLUMN`) for older
    `social.db` files.
  - New bounds: `MAX_RELATIONSHIP_EVENT_TYPE_CHARS = 64`,
    `MAX_RELATIONSHIP_EVENT_DETAIL_BYTES = 8192`.
- `crates/aegis-social/src/awareness.rs` (new)
  - `SocialAwarenessSummary`, `AwarenessMembership`, `AwarenessRelationship`,
    `social_awareness_summary`, `social_awareness_summary_limited`, and
    `SocialAwarenessSummary::build` / `build_limited`.
  - Bounds: `MAX_AWARENESS_MEMBERSHIPS = 64`,
    `MAX_AWARENESS_RELATIONSHIPS = 64`, `MAX_AWARENESS_TAGS = 8`.
- `crates/aegis-social/src/lib.rs`
  - Re-exports the additive awareness surface and new event constants.
- `crates/aegis-social/tests/awareness.rs` (new, 6 tests)
- `docs/implementation/AEGIS_FINAL_AGENT13_REPORT.md` (this file)

## Audit results

- **Group membership:** every group read/post/member/event/summary operation
  calls the live membership check; the owner retains administrative access.
  Removal is therefore immediate because no cached membership is consulted.
- **Private context:** `context_for_core` is membership-gated and returns only
  messages from the shared group table. It never opens `social.db`, so private
  relationship `label`, `trust`, `notes`, or `tags` cannot enter a group read.
- **Relationship ownership:** each `RelationshipStore` is opened inside the
  owning Core directory, bound to the immutable Core UUID and database instance
  UUID. A copied foreign `social.db` is rejected with `OwnershipMismatch`.
- **History:** relationship events and group events are append-only by SQLite
  triggers (UPDATE/DELETE abort). Events are read per relationship in insertion
  order; explicit recording is transactional with the revision increment.
- **Membership history:** group events record `group.created`, `member.added`,
  `member.removed`, and `message.posted`; the participation summary reflects
  current membership only, so a departed Core's old groups are not reconstructed.

## Additive API

### Explicit relationship events

```rust
let event = store.record_event(
    RelationshipTargetKind::Core,
    target_core_uuid,
    "interaction.shared_activity",
    serde_json::json!({"activity": "walk"}),
)?;
// event.revision == previous relationship revision + 1
```

- Caller supplies target, event type, and JSON detail; nothing is inferred.
- The relationship row and the event are updated/appended in one transaction.
- Event type is bounded to 64 chars with no control characters; serialized
  detail is bounded to 8 KiB.
- A missing relationship returns `SocialError::MemberNotFound`.
- `events()` returns each event with its recorded revision.

### Bounded per-Core social-awareness summary

```rust
let summary = social_awareness_summary(&groups, &relationship_store)?;
// or: social_awareness_summary_limited(&groups, &store, 16, 16)?
```

- `core_uuid` comes from the ownership-bound relationship store.
- Memberships come only from the group store's live participation query.
- Relationships come only from that Core's own `social.db`.
- `membership_count` / `relationship_count` are pre-truncation totals;
  `memberships_truncated` / `relationships_truncated` report cuts.
- `notes` are intentionally omitted; tags are capped at 8 entries.
- No trust, familiarity, awareness, or interaction is synthesized. Every field
  is copied from a stored membership row or an explicit relationship record.

## Verification

- `cargo test -p aegis-social` - **23 passed, 0 failed, 0 ignored**
  - `tests/social.rs`: 17 (existing)
  - `tests/awareness.rs`: 6 (new)
  - lib unit tests: 0; doc-tests: 0
- New tests:
  - `explicit_relationship_events_are_append_only_and_revisioned`
  - `explicit_relationship_events_validate_and_require_a_relationship`
  - `relationship_event_histories_are_independent_per_core`
  - `awareness_summary_is_membership_scoped_and_non_members_are_denied`
  - `awareness_summaries_never_include_other_cores_private_relationships`
  - `awareness_summary_reports_only_recorded_data_and_is_bounded`
- `cargo clippy -p aegis-social --all-targets -- -D warnings` - clean.
- `cargo fmt -p aegis-social` applied.

## Privacy guarantees (tested)

- A non-member is denied `context_for_core`, `messages`, and
  `activity_summary`; a non-member's group is absent from their awareness
  summary.
- Core A's awareness summary never contains Core B's private relationship
  label, notes, or tags (and vice versa), even when both are members of the same
  group.
- Relationship event histories are independent per Core; a store cannot see the
  other Core's events, and foreign database copies are rejected by ownership.
- Group projections never contain per-Core relationship fields; `notes` are not
  exposed by the awareness summary at all.
- Relationship events are append-only: direct SQL UPDATE/DELETE is rejected by
  triggers.

## Limitations

- Bounded summaries bound the returned vectors, but the participation/list
  queries materialize all current memberships and relationships before
  truncation; there is no SQL pagination for very large rosters.
- `events()` remains an unbounded per-relationship read (pre-existing API); the
  new explicit recording path is bounded per event.
- Revisions are per-relationship monotonic integers, not a hash chain or signed
  log; an attacker with write access to the Core database could rewrite the
  whole file. Ownership binding and append-only triggers are integrity
  guardrails, not tamper-proof authenticity.
- The awareness summary omits free-form notes and per-relationship event detail;
  callers needing history use `events()` explicitly.
- Relationship events are not automatically wired into emotion, conversation, or
  backup flows; Agent 05's explicit influence path remains caller-driven.
- No cross-Core relationship awareness is provided by design. A Core only sees
  its own private relationship records plus shared group data it is entitled to.
