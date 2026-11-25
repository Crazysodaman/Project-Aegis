# Batch 1 — Wave 1a Implementation Log

**Date:** 2026-11-22  
**Target:** `D:\Aegis` only  
**Reference repository:** `D:\SofiaDev\SofiaAdaLyra` (strictly read-only)  
**Scope:** Wave 1a platform foundation plus shared contracts. Wave 1b–1d and Wave 2/3 were not started.

## Owner-approved decisions applied

- **Q1:** Rust-first architecture.
- **Q2:** Wave 1a only, plus shared contract scaffold.
- **Q3:** Local JSON-RPC over Windows named pipes / Unix domain sockets.
- **Q4:** Plug-in owner verifier with a DPAPI-backed Windows development provider.
- **Q5:** Separate SQLite databases with OS-level permissions; encryption deferred.
- **Q6:** Configurable local paths and disposable tests; A-04/B-01/B-06 remain provisional.
- **Q7:** Local Git repository initialized at `D:\Aegis`.

## Implemented

### Workspace

- Rust 2021 workspace pinned to Rust 1.88.0.
- Crates: `aegis-contracts`, `aegis-storage`, `aegis-safe`, `aegis-platform`, `aegis-ipc`, `aegis-cli`.
- `rust-toolchain.toml`, `.gitignore`, README, JSON Schemas under `contracts/ipc/v1`.

### `aegis-contracts`

- Versioned IPC request/response envelopes (`aegis.ipc.request.v1`, `aegis.ipc.response.v1`, protocol `1.0`).
- Frame-size, schema, protocol, identifier, client, token, and parameter validation.
- Platform status, STOP status, STOP mutation, and effect-ping contracts.
- Core lifecycle vocabulary and roster policy constants: approved `1,2,3,6,7`; prohibited `4,5`.

### `aegis-storage`

- Absolute-root storage path policy.
- Distinct `platform.db` and `safe.db` paths plus `owner.proof`.
- SQLite application markers: `AEGP` for Platform, `AEGS` for Safe.
- Schema revision table and revision mismatch rejection.
- Read-only open helper, file permissions on Unix, symlink/redirect rejection at root.
- No domain tables and no authorization policy.

### `aegis-safe`

- Separate Safe SQLite database with `safe_stop`, `safe_owner`, and append-only `safe_audit`.
- DPAPI current-user protection on Windows; explicit 0600 Unix development fallback.
- Owner proof initialization, verification, and local token loading.
- STOP set/clear with owner authorization and monotonic revision.
- Hash-chained audit with validity verification.
- Owner proof is required for STOP changes; STOP read/clear remain possible while STOP is active.

### `aegis-platform`

- Core-neutral installation identity (`AEGIS-PLATFORM-*`) with no Sofía identity or personality data.
- Zero-Core startup and status (`zero_core`, `core_count=0`, `authority=local_single`).
- Empty `platform_core` registry restricted to Core numbers `1,2,3,6,7`.
- Separate Platform and Safe databases; no cross-owned tables.
- Owner-authorized dispatch for status, STOP status/set/clear, and a protected no-op effect.
- STOP is checked at effect execution time, not only startup.
- No Core private data or authorization influenced by personality, memory, autonomy, or relationships.

### `aegis-ipc`

- Local JSON-RPC over `interprocess` namespaced sockets (Windows named pipes; Unix domain sockets).
- Newline-framed JSON with maximum frame size.
- Strict protocol validation, malformed-JSON handling, unauthorized-token handling, and STOP error mapping.
- Bounded request-serving for tests and local smoke checks.

### `aegis-cli`

- `init`, `status`, `safe status|set|clear`, `effect ping`, `serve`, and `ipc ...` commands.
- JSON output.
- Development owner token is written to the Safe proof file; `init --print-owner-token` prints it only when explicitly requested.

## Evidence

Environment: Windows 11, PowerShell 5.1, Rust/Cargo 1.88.0, Python 3.12.10, D:\Aegis.

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | Pass |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| Unit/integration tests | `cargo test --workspace` | **30 passed, 0 failed** |
| CLI build | `cargo build -p aegis-cli` | Pass, `target\debug\aegis.exe` |
| CLI smoke | `aegis init`, `status`, `safe set`, `effect ping`, `safe clear` | Pass; STOP blocked effect and clear restored it |
| IPC smoke | `aegis serve --requests 1` + `aegis ipc status` | Pass over a named pipe |

Test breakdown:

- `aegis-contracts`: 6 unit tests.
- `aegis-ipc`: 5 integration tests.
- `aegis-platform`: 6 integration tests.
- `aegis-safe`: 2 unit + 6 integration tests.
- `aegis-storage`: 5 unit tests.

Classification:

- **Coded:** all listed components.
- **Built:** `cargo build -p aegis-cli` succeeded; workspace test binaries built.
- **Unit-tested:** contracts, storage, Safe owner proof.
- **Integration-tested:** Platform, Safe STOP/audit, IPC round-trip/authorization/failure handling.
- **Runtime-tested locally:** CLI and named-pipe IPC smoke on `D:\Aegis\var\batch1-smoke`.
- **Live-verified:** not claimed.
- **Owner-accepted:** not claimed.

## Deliberate limitations

- Windows owner proof uses DPAPI for the current user. The Unix fallback (`unix-file-dev`) is a 0600 development file, not production owner authentication.
- Local direct CLI commands load the development owner token from the proof file. The threat model assumes a trusted same-user local environment; this is not strong protection against same-user malware.
- Encryption at rest is deferred (Q5-A). Platform/Safe files rely on OS permissions/inherited ACLs.
- No Platform daemon or OS service is installed. `serve` handles a bounded number of requests and exits.\n- Windows DPAPI and Windows named-pipe paths were built and tested. Unix `cfg` paths are implemented but were not cross-compiled or tested on Linux in this environment.
- No Core database, Core activation, memory, relationships, consent, cognition, chat, voice, avatar, or TUI is implemented in Batch 1.
- No backup target, retention, RPO/RTO, or final host placement is finalized. **A-04, B-01, and B-06 remain provisional.**
- Adult capability remains fail-closed; no adult capability is implemented or activated.
- No legacy Sofía database, asset, service, task, or repository file was opened writable, migrated, modified, or deleted.
- No Git commit or push was performed.

## Next milestone

Wave 1b is not started. It requires owner approval and should cover:

- Full per-Core database composition and backup manifest.
- Canonical LF-normalized Constitution digest alignment and `fresh_no_migration`.
- Owner/peer relationship ledger and scoped consent/capability state.
- Provenance-first memory, deletion/tombstones, restart continuity, private chat isolation, and provider-neutral routing.
