# Batch 5 — Agent 09: Social System & Module Execution

Baseline: `2bd2513`. Work is additive and limited to `crates/aegis-social` and
`crates/aegis-modules`. No existing checks were weakened.

## Scope

This batch adds explicit, membership-gated Core group-context reads, a bounded
group activity summary, an aggregated module execution audit, and real
process-based tests for module execution and lifecycle/permission enforcement.

## Files changed

- `crates/aegis-social/src/groups.rs`
- `crates/aegis-social/src/lib.rs`
- `crates/aegis-social/tests/social.rs`
- `crates/aegis-modules/src/runtime.rs`
- `crates/aegis-modules/src/lib.rs`
- `crates/aegis-modules/tests/runtime.rs`
- `docs/implementation/BATCH5_AGENT09_SOCIAL_MODULES.md` (this file)

## Social: Core group context

`GroupStore::context_for_core(core_uuid, group_id, limit)` returns recent
group-visible messages (`GroupMessage` with `sender_kind` and `sender_id`) in
chronological order, bounded to `1..=MAX_GROUP_MESSAGES`.

- Calls `SocialActor::core` and re-checks membership on every invocation.
- A non-member receives `SocialError::NotMember`.
- A removed member is denied immediately on the next call.
- The projection (`CoreGroupContext`) contains only group messages. It never
  joins or reads the per-Core relationship database.

Per-Core relationships remain isolated in `<core>/social.db`, bound to the Core
UUID and database instance UUID. Group reads do not expose relationship
`label`, `trust`, `notes`, or `tags` to any other Core.

## Social: group activity summary

`GroupStore::activity_summary(group_id, actor)` returns a bounded, serializable
`GroupActivitySummary` with:

- `member_count`
- `message_count`
- `last_message_id` / `last_message_at`
- `last_activity_at` and `revision`

Access uses the same owner/member rule as other group reads. The summary reports
only counts and timestamps already owned by the group store. It adds no trust,
awareness, sentiment, or relationship inference.

## Modules: execution audit summary

`ModuleRuntime::execution_summary()` aggregates `ModuleRuntime::list()` into a
`ModuleExecutionAudit`:

- `module_count`, `enabled_count`, `running_count`, `running_modules`
- `starting_count`, `stale_count`, `failed_count`, `stopped_count`
- heartbeat buckets: fresh / stale / missing
- oldest and freshest heartbeat age
- the most recent recorded/derived execution error (`ModuleAuditError`)

The audit is read-only. It starts, stops, or mutates nothing, and it never treats
a registry row as proof of a live process. Effective state still derives from
process liveness plus heartbeat freshness.

## Modules: lifecycle and permission enforcement

`run_module_tool` continues to require lifecycle state `enabled` before resolving
the package entrypoint or spawning the module. The new integration test proves,
with the real example module binary:

1. `installed` → `ModuleToolError::NotEnabled`
2. `enabled` → tool runs and returns the expected payload
3. `disabled` → `ModuleToolError::NotEnabled`
4. `uninstalled` → `ModuleToolError::NotEnabled`

No authorization, lifecycle, or manifest checks were relaxed.

## Real execution evidence (not simulated)

Integration tests build and use `crates/aegis-modules/src/bin/aegis-example-module.rs`
through Cargo's `CARGO_BIN_EXE_aegis-example-module` variable. The example
process implements the documented contract: heartbeat file writes, stop-sentinel
handling, optional failure switch, and one-shot tool mode.

If the binary needs to be built manually:

```powershell
cargo build -p aegis-modules --bin aegis-example-module
```

Tests exercise actual process start, healthy heartbeat, graceful stop, crash
detection (`--fail-after-ms 300`), forced stop (`--ignore-stop`), and real tool
execution.

## Verification

```powershell
cargo test -p aegis-social -p aegis-modules
cargo clippy -p aegis-social -p aegis-modules --all-targets -- -D warnings
```

Result at time of writing:

- `aegis-modules`: 33 tests passed (5 unit, 16 `modules`, 12 `runtime`)
- `aegis-social`: 11 tests passed (`social`)
- Total: 44 passed, 0 failed
- Clippy: clean with `-D warnings`

Six tests were added: three social tests and three module runtime tests.

## Supported functionality

- Platform-owned private groups with owner/member enforcement.
- Core-scoped recent group-context reads and bounded group activity summaries.
- Per-Core private relationship persistence with ownership binding.
- Module manifest validation, content-digest verification, package
  materialization, and lifecycle transitions.
- Supervised process start/stop/restart, heartbeat freshness, crash detection,
  forced termination, and one-shot module tools for `process`/`native`
  entrypoints.
- Aggregated execution audit for status/UI consumers.

## Unsupported integrations and deliberate limitations

- OS-level sandboxing is **not implemented**. Modules are trusted local code.
  There is no seccomp/AppContainer/Job Object, filesystem jail, network
  isolation, or capability dropping.
- Declared resource limits (`max_memory_bytes`, `max_cpu_millis`,
  `max_disk_bytes`, `max_open_files`) are validated manifest fields only; they
  are **not enforced** at runtime.
- `wasm` entrypoints are accepted by the manifest schema but the runtime rejects
  them as unsupported. Only `process` and `native` are executed.
- Module lifecycle hooks are declared and validated but not invoked.
- There is no publisher signature verification or installation-trust model. The
  manifest digest proves content consistency only.
- No remote registry, marketplace, auto-update, hot reload, or module-to-module
  IPC is implemented.
- No group attachments, media, message editing/deletion, read receipts,
  notifications, federation, or external messenger bridges are implemented.
  Group messages and events are immutable/append-only.
- No cross-Core relationship sharing is implemented by design. Relationship data
  stays in each Core's own database and is never exposed through group reads.
- No social awareness, trust inference, personality, sentiment, or relationship
  state is fabricated by any new API.
- Process liveness/termination currently relies on platform commands
  (`tasklist`/`taskkill` on Windows, `kill` elsewhere); there is no dedicated
  job-object kill-tree or portable supervision daemon.
- Child processes receive a cleared environment and no owner token/Platform
  secrets, but this is environment hygiene, not an isolation boundary.

## Next steps

- Add an OS-level sandbox/resource enforcement layer if untrusted modules become
  a requirement.
- Implement WASM entrypoint support or explicitly remove it from the accepted
  manifest contract.
- Execute declared lifecycle hooks with the same audited process boundary.
- Add signatures/attestations if publisher authenticity is required.
- Add UI/TUI consumers for the new activity summary and execution audit.
