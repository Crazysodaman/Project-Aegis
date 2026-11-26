# Aegis Final - Agent 19 Report (Integration, Performance & Reliability)

- Repo: `D:\Aegis`
- Baseline HEAD: `2bd2513`
- Scope: additive reliability/performance regression tests only. No production source was edited, and no files outside the allowed test directories plus this report were changed.
- No new dependencies. No live `var/` use. No external network. No sleeps.

## Files added

| File | New tests |
|---|---:|
| `crates/aegis-platform/tests/integration_reliability.rs` | 4 |
| `crates/aegis-core-sdk/tests/integration_reliability.rs` | 3 |
| `crates/aegis-conversation/tests/integration_reliability.rs` | 4 |
| `crates/aegis-embodiment/tests/integration_reliability.rs` | 3 |
| `crates/aegis-backup/tests/integration_reliability.rs` | 3 |
| **Total** | **17** |

All tests use tempdirs and fake/in-process providers only.

## Test coverage and counts

### Platform (`aegis-platform`, 4 tests)
- `concurrent_cores_share_one_identical_world_snapshot_without_contamination` (8 concurrent readers, one provider call, identical snapshots).
- `concurrent_stale_refresh_documents_duplicate_provider_work` (4 synchronized stale racers; records duplicate fetches).
- `failing_weather_degrades_gracefully_but_re_polls_on_every_stale_refresh` (honest unavailable snapshot, bounded provenance note, repeated failure polling).
- `large_but_bounded_provider_inputs_are_truncated_by_the_snapshot_contract` (200 provider forecast entries truncated to 16).

### Core SDK (`aegis-core-sdk`, 3 tests)
- `concurrent_multi_core_personal_access_is_isolated` (4 Cores, 6 writes each, per-DB owner/count checks).
- `repeated_open_reopen_cycles_do_not_leak_or_corrupt_stores` (25 open/apply/experience/memory/reopen cycles, then temp-root removal succeeds).
- `large_but_bounded_inputs_are_accepted_and_summaries_stay_bounded` (64 KiB memory, 512-char experience/thread/session, oversize rejection, continuity caps 8/8).

### Conversation (`aegis-conversation`, 4 tests)
- `repeated_context_injection_keeps_prompt_bounded_while_history_grows` (40 turns with world/continuity/emotion/activities/memories; constant prompt length, history capped, transcript grows).
- `large_but_bounded_user_and_instruction_inputs_are_capped` (60k-char whitespace-delimited user text and 30k instructions; prompt and history bounded; >message-max rejected).
- `single_long_token_leaves_an_orphan_user_message_after_retrieval_rejects_it` (60k single token within message bound; retrieval rejects, user message persists).
- `concurrent_multi_core_conversations_do_not_contaminate_each_other` (2 Cores x 5 turns; exports are marker-clean and cross-Core session lookup fails).

### Embodiment (`aegis-embodiment`, 3 tests)
- `repeated_open_reopen_and_audit_cycles_release_handles` (12 cycles, verified audits, temp-root removal succeeds).
- `concurrent_multi_core_asset_libraries_stay_isolated` (4 Cores x 5 assets, no cross-Core search hits).
- `large_but_bounded_assets_and_metadata_stay_within_contract` (1 MiB asset, ~60 KB metadata, 128-char name accepted; 129-char name, ~70 KB metadata, empty bytes rejected).

### Backup (`aegis-backup`, 3 tests)
- `repeated_backup_verify_restore_cycles_preserve_identity_and_content` (3 chained backup -> verify -> restore cycles; stable `platform_id`, Core integrity verified each hop).
- `repeated_restore_over_allowed_existing_target_keeps_verified_content` (3 repeated restores with preserved rollback; backup remains verified).
- `backup_directory_discovery_is_stable_across_repeated_cycles` (3 snapshots; sorted discovery stable).

## Verification

- `cargo test -p aegis-platform -p aegis-core-sdk -p aegis-conversation -p aegis-embodiment -p aegis-backup` - all suites passed, including all 17 new tests.
- `cargo clippy -p aegis-platform -p aegis-core-sdk -p aegis-conversation -p aegis-embodiment -p aegis-backup --tests -- -D warnings` - clean.

## Coarse timing observations (measured, not asserted)

- Platform: 8 concurrent fresh world reads ~10 ms.
- Core SDK: 4 Cores x 6 personal writes ~1.9 s; 25 open/reopen cycles ~3.1 s.
- Conversation: 40 context-heavy turns ~3.6 s (system prompt 2,689 chars, cap 8,000); 2 Cores x 5 concurrent turns ~3.3 s.
- Embodiment: 12 open/audit cycles ~1.9 s; 4 Cores x 5 asset imports ~1.1 s.
- Backup: 3 chained backup/verify/restore cycles ~1.2 s; 3 restores over an existing target ~1.0 s.

These are coarse diagnostics only; no flaky wall-clock threshold is asserted.

## Findings

- Fresh shared world snapshots are correctly reused across concurrent Cores: 8 readers caused one provider call and all received byte-identical snapshots.
- Persistent stores reopen cleanly and release handles: removing the temp root after dropping stores succeeded on Windows for Core SDK and embodiment. Backup/restore temp roots also removed successfully.
- Prompts are bounded under sustained context injection: 40 context-heavy turns kept the system prompt constant at 2,689 chars while persisted history grew to 80 messages and request history stayed capped.
- Large-but-bounded data is handled at the contract edges: forecast truncation to 16, 64 KiB memory entries, 512-char personal text fields, 1 MiB assets, ~60 KB asset metadata, and 60k-char conversation messages all behave as bounded.
- Multi-Core isolation held concurrently for world snapshots, personal state, conversation transcripts, and embodiment assets.

## Risks / duplicate work and polling (reported, not fixed)

1. **Concurrent stale refresh duplicates provider work.** `WorldService::refresh_if_stale_at` reads `latest_at`, then fetches, then writes without a coalescing lock. With 4 synchronized stale racers the test observed 4 provider calls. A scheduled multi-Core refresher can multiply provider/network load.
2. **Unavailable weather is always "stale".** Any snapshot without weather is `FreshnessState::Unavailable`, so every subsequent refresh re-polls the provider and appends a new `world_events` row. The failing-provider test observed calls equal to every refresh round. There is no failure TTL/backoff, so a persistent outage causes repeated work and linear history growth.
3. **World history is append-only per refresh.** Even successful refreshes append one history row per snapshot. Tests show per-call linear growth only; no retention/compaction is exercised here.
4. **Keyword retrieval term is not length-capped to the query limit.** A 60,000-char unbroken user token (below `MAX_MESSAGE_CHARS`) becomes one search term and is rejected by `MemoryQuery` validation. Because `send_turn` appends the user message before building the request, the failed turn leaves an orphan user-only message and no assistant reply.
5. **Concurrent SQLite writers can see lock contention.** The concurrent stale-refresh test tolerates per-racer write errors while confirming the store remains valid; `open_initialized` uses `journal_mode=DELETE` with no explicit busy timeout. A future coalescing design should serialize or retry refresh writes.

No source changes were made for any of the above.
