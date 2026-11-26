# Aegis FINAL — Agent 01 (Platform Core & Lifecycle)

**Status:** Audited; 3 lifecycle integration tests added; `cargo test -p aegis-platform`
and `cargo clippy -p aegis-platform --all-targets -- -D warnings` pass.
No source, Cargo, CLI, or non-owned files changed. No commit/deploy.

## Files
- `crates/aegis-platform/tests/platform_lifecycle.rs` (new, 3 tests)
- `docs/implementation/AEGIS_FINAL_AGENT01_REPORT.md` (this report)

## Audited implementation (source)

### Startup / identity / status
- Initialization creates the Platform SQLite database, identity, module registry,
  Safe store, and owner proof: `crates/aegis-platform/src/service.rs:79-96`.
- `start()` opens paths, re-initializes store/modules, and starts Safe; it does not
  create the owner proof (that is `initialize()`): `service.rs:98-111`.
- Host identity is a persisted singleton row (`platform_id`, `callsign`,
  `created_at`); callsign is `AEGIS-PLATFORM-<first 8 UUID chars>`:
  `crates/aegis-platform/src/store.rs:33-38`, `store.rs:56-68`, `store.rs:106-124`.
- Status is assembled from identity + Core count + Safe stop state; `zero_core` is
  exactly `core_count == 0`; `authority` is hard-coded `"local_single"`;
  `generated_at` is wall-clock RFC 3339: `service.rs:117-132`.
- Status contract and IPC constants: `crates/aegis-contracts/src/status.rs:11-21`,
  `crates/aegis-contracts/src/ipc.rs:14-22`.

### Dispatch / shutdown
- Every `dispatch` call verifies the owner token first; supported methods are
  status, shutdown, effect ping, and Safe stop status/set/clear: `service.rs:138-197`.
- `platform.shutdown` only sets an in-process `AtomicBool`: `service.rs:149-152`,
  `service.rs:75`, `service.rs:109`, `service.rs:228-235`.
- The IPC server exits its accept loop when that flag is observed:
  `crates/aegis-ipc/src/lib.rs:45-59` (check at line 51).
- Detached process lifecycle (spawn, IPC readiness, graceful stop + forced kill) is
  in a separate crate: `crates/aegis-service/src/lib.rs:241-303` (start),
  `lib.rs:306-350` (stop), `lib.rs:195-238` (status).

### Registry / Core discovery
- Platform store creates legacy `platform_core` (empty is valid): `store.rs:39-46`.
- `core_count()` prefers `core_registry` whenever that table exists, otherwise
  falls back to `platform_core`, and counts every row: `store.rs:80-103`
  (detection `86-97`, count `98-103`).
- Modern registry schema, registration, listing, lifecycle, and legacy migration
  live in `crates/aegis-core-registry/src/lib.rs:87-115` (schema),
  `:118-184` (register), `:193-205` (list), `:235-301` (transitions),
  `:347-427` (legacy migration).
- Install orchestration writes Core storage/integrity then registers:
  `crates/aegis-core-install/src/lib.rs:120-160`, `:215-223`; list/get at `:343-349`.

## Tests added (`platform_lifecycle.rs`)
1. `zero_core_start_status_shutdown_and_reopen_keep_host_identity` — zero-Core status
   (schema/protocol/authority/identity/callsign), owner token, dispatch status,
   `platform.shutdown` flag, and identity/owner stability after reopen.
2. `multi_core_discovery_prefers_core_registry_and_survives_reopen` — 0 → 2 legacy
   `platform_core` rows → 3 `core_registry` rows; verifies registry precedence and
   that discovery survives reopen.
3. `shutdown_does_not_clear_persisted_stop_state_across_reopen` — STOP persists in
   Safe across a shutdown-flag request and reopen, blocks effects, then clears.

## Verification
- `cargo test -p aegis-platform` → **38 passed, 0 failed** (new file: 3 passed).
- `cargo clippy -p aegis-platform --all-targets -- -D warnings` → **exit 0**.
- All tests use `tempfile` roots only; no live `var/` or Core data touched.

## Implemented and runtime-tested
- Zero-Core platform start, status, identity/callsign persistence, owner token.
- Status via direct API and owner-gated `platform.status` dispatch.
- In-process shutdown request/idempotence and reset on reopen.
- Multi-Core count discovery from both registry tables, precedence, reopen stability.
- STOP persistence across shutdown/reopen (existing Safe behavior confirmed).

## Production gaps (exact)
1. **Shutdown is not a process lifecycle.** `platform.shutdown` only flips
   `AtomicBool` (`service.rs:149-152`, `228-235`); it is not persisted, not
   signalled to the OS, and resets on reopen. Detached start/stop/kill is
   `aegis-service` (`lib.rs:241-350`) and is not exercised by Platform tests.
2. **Shutdown is not wake-capable.** The IPC loop only checks the flag between
   `accept()` calls (`aegis-ipc/src/lib.rs:51-55`); if shutdown is requested
   out-of-band, a blocked `accept()` does not wake, so exit can stall until the
   next connection.
3. **Discovery is count-only.** `PlatformService` exposes no per-Core enumeration,
   lifecycle detail, storage path, or integrity/health. Real listing lives in
   `aegis-core-registry` (`:193-205`) and `aegis-core-install` (`:343-349`) but is
   not wired into `PlatformStatus` (`status.rs:11-21`).
4. **Count ignores lifecycle.** `core_count()` counts all rows regardless of
   `design_only`/`installed`/`active`/`suspended`/`retired` (`store.rs:98-103`), so
   `zero_core`/`core_count` can overstate usable Cores.
5. **Modern registry is external.** A fresh Platform creates only legacy
   `platform_core` (`store.rs:39-46`); `core_registry` exists only after
   `CoreRegistry::initialize` (`aegis-core-registry/src/lib.rs:87-115`) is called by
   another layer, so the preferred discovery table is often absent.
6. **No filesystem/manifest discovery.** Discovery is SQLite row counting only; no
   scan of `cores/` or integrity manifests (`store.rs:80-103`).
7. **No concurrency/idempotency guard.** Identity creation is check-then-insert
   outside a transaction (`store.rs:53-68`); two concurrent initializers can race.
8. **Service status is elsewhere.** `ServiceStatus` (`aegis-service/src/lib.rs:119-127`,
   `:195-238`) is not part of `PlatformStatus` or platform IPC; liveness relies on
   external `tasklist`/`kill -0` (`:441-469`) with no PID-reuse protection.
9. **Host identity is local-only.** `authority` is hard-coded `"local_single"`
   (`service.rs:129`); identity has no machine binding, rotation, or attestation,
   and the callsign derives from only 8 UUID characters (`store.rs:57`).
10. **No readiness/health state.** `status()` is a snapshot (`service.rs:117-132`);
    there is no liveness of registry rows, Core runtimes, or Safe audit health.

## Blockers / not runtime-tested here
- Detached `aegis-service-host` start/stop/IPC probes (would require the
  `aegis-service`/host binaries and a live local socket).
- Actual OS process exit caused by `platform.shutdown`.
- Per-Core discovery/integrity through public registry/install APIs from this crate
  (not a declared dependency of `aegis-platform`; no Cargo edits allowed).
- Multi-process/concurrent-initialize contention and Windows permission/DPAPI paths.

**Uncommitted work:** preserved; only the two owned files above were created/changed.
