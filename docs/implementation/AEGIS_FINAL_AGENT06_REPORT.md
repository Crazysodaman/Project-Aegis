# Aegis FINAL — Agent 06 (Memory & Conversation) Report

Status: **Complete**
Baseline HEAD: `2bd2513`
Scope: audit memory kinds / ownership / retrieval / eviction and session
persistence; add an additive bounded session summary and an explicit,
idempotent important-experience → memory consolidation helper.

No new dependencies. No changes to `engine.rs`, `aegis-core-sdk/src/lib.rs`,
the CLI, workspace/Cargo files, or any other crate. Transcripts are never
dumped: the summary is metadata-only and consolidation only stores the
experiences the caller explicitly supplies.

## Files changed

- `crates/aegis-core-sdk/src/memory.rs` — add `CoreMemoryStore::find_by_source`.
- `crates/aegis-conversation/src/store.rs` — add `SessionSummary`,
  `ImportantExperience`, `ConsolidationOutcome`, bounded constants,
  `ConversationStore::session_summary`, `ConversationStore::consolidate_experiences`,
  and stable-key helpers.
- `crates/aegis-conversation/src/lib.rs` — re-export the additive surface.
- `crates/aegis-conversation/tests/memory_consolidation.rs` (new) — 7 tests.
- `docs/implementation/AEGIS_FINAL_AGENT06_REPORT.md` (this file).

## Audit findings

### Memory kinds
- Fixed `MemoryKind` set: `fact`, `preference`, `episode`, `reflection`,
  `relationship`, `goal`, plus a bounded custom `Other` (ASCII alphanumeric,
  `_`, `-`; ≤64 chars).
- Kind strings are stored as text but re-parsed and validated on every read and
  import, so unknown/corrupt kinds are rejected rather than propagated.

### Ownership
- `memory.db` is opened only through `CoreStorageLayout` inside the owning Core
  directory, with canonical-path and symlink checks, an immutable singleton
  ownership row, and a Core identity snapshot check. Copying another Core's DB
  is rejected (`OwnershipMismatch` / `OwnershipMissing`).
- `conversation.db` binds both the immutable `core_uuid` and the Core's
  `database_instance_uuid` in an immutable singleton row; foreign copies are
  rejected. Every query is scoped by `core_uuid` (or by the session's owning
  Core).
- Consolidation refuses to write when the supplied memory store belongs to a
  different Core, before any mutation.

### Retrieval
- `KeywordRetriever` is honest: ≤8 distinct terms, ≤50 results, id-based
  dedupe, and it never claims semantic relevance.
- `EmbeddingRetriever` considers ≤500 live candidates, ranks by cosine
  similarity, caps output at 50, caches vectors by content digest, and reports
  the method actually used. Provider/cache failure falls back to keyword and
  says so.
- Default search excludes archived and soft-deleted entries; archived/deleted
  data remains available explicitly.

### Eviction / retention
- Memory eviction is explicit only: `archive`, `restore_archived`, `soft_delete`,
  `purge`, with append-only event history. Import is bounded by entry/event
  counts and content size.
- No automatic age/size eviction exists for memory entries or the embedding
  vector cache. Personal experiences are append-only with a bounded read
  window (8). This is an intentional but documented gap.

### Session persistence
- Per-Core `conversation.db`; sessions have `active`/`archived`/`deleted`
  states and a monotonic revision. `conversation_events` is append-only by
  trigger. Messages are immutable through the public API.
- The new summary derives counts from stored messages and cross-checks them
  against the session's maintained `message_count`; drift is reported as
  corrupt data instead of a plausible summary.

## Additive API

- `CoreMemoryStore::find_by_source(&str) -> Vec<MemoryEntry>`
  - Exact `core_uuid` + `source` match, oldest first, including archived and
    soft-deleted records (used for idempotency checks).
- `ConversationStore::session_summary(&str) -> SessionSummary`
  - Bounded, metadata-only: state, counts per role, first/last message
    timestamps, created/updated/revision. No message content.
- `ConversationStore::consolidate_experiences(&CoreMemoryStore, session_id, &[ImportantExperience]) -> ConsolidationOutcome`
  - Explicit; caller designates importance. No transcript scanning.
  - Owned-Core check, session existence check, batch cap `MAX_CONSOLIDATION_BATCH`
    (32), per-experience char cap `MAX_CONSOLIDATION_CONTENT_CHARS` (512).
  - Writes `episode` memories with confidence `0.75`, provenance
    `conversation.consolidation:<session>`, tags `consolidation` and
    `session:<id>`.
  - Stable source key: `conversation.experience:<session>:<fnv1a64(content)>`.
    Repeated, reordered, and in-batch duplicate calls are idempotent; archived
    or soft-deleted consolidated entries still count as already consolidated.

## Verification

- `cargo test -p aegis-core-sdk -p aegis-conversation` — **63 passed, 0 failed**
  - `aegis-conversation`: 30 total (lib 1, `batch8_qa` 3, `conversation` 13,
    `memory_consolidation` 7 new, `world_integration` 6)
  - `aegis-core-sdk`: 33 total (lib 0, `batch8_qa` 4, `personal` 12,
    `personal_agent05` 3, `sdk` 14)
  - Doc-tests: 0
- New tests (7):
  - `consolidation_creates_one_memory_and_preserves_ownership`
  - `repeated_and_reordered_consolidation_is_idempotent`
  - `archived_consolidation_still_counts_as_already_consolidated`
  - `consolidation_rejects_a_foreign_core_memory_store`
  - `consolidation_is_bounded_and_skips_empty_entries`
  - `session_summary_is_metadata_only_and_bounded`
  - `session_summary_is_isolated_per_core`
- `cargo clippy -p aegis-core-sdk -p aegis-conversation --all-targets -- -D warnings`
  — clean.

## Gaps / limitations

- No automatic eviction or retention policy for memory entries or cached
  embedding vectors; storage grows until an explicit archive/soft-delete/purge.
- Keyword search filters content in Rust after loading the Core's rows instead
  of using SQL `LIKE`/FTS, so each search term is O(entries in the Core). It is
  bounded per Core but not a large-scale search guarantee.
- `export_session` / `export_archive` are capped by `MAX_MESSAGES_PER_READ`
  (500) per session and have no pagination, so sessions with more than 500
  messages export only the first 500. `list_sessions` itself is unbounded.
- The stable key hashes the (possibly truncated) content with 64-bit FNV-1a.
  Two experiences that share the same first 512 characters collide and are
  treated as one; the key is session-scoped, so identical text in different
  sessions produces distinct memories.
- Consolidation is explicit only: there is no importance scoring, cross-session
  deduplication, or automatic summarization of transcripts.
- Memory, summaries, and consolidated episodes remain descriptive context and
  are never authorization inputs.
