# Batch 1 — Test and Runtime Evidence

**Date:** 2026-11-22  
**Host:** Windows 11 Home, PowerShell 5.1  
**Toolchain:** Rust/Cargo 1.88.0 (`x86_64-pc-windows-msvc`)  
**Target:** `D:\Aegis`  
**Evidence class:** Local development only. No production host, live service, or owner acceptance.

## Commands and results

| # | Command | Result |
|---|---|---|
| 1 | `cargo fmt --all -- --check` | Pass |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| 3 | `cargo test --workspace` | **30 passed, 0 failed** |
| 4 | `cargo build -p aegis-cli` | Pass; `target\debug\aegis.exe` produced |
| 5 | `aegis init --root D:\Aegis\var\batch1-smoke` | Pass; Platform ID and callsign produced; no owner token printed |
| 6 | `aegis status --root ... --json` | Pass; `zero_core=true`, `core_count=0`, `stop_active=false` |
| 7 | `aegis safe set --root ... --reason "smoke test"` | Pass; STOP active, revision 1 |
| 8 | `aegis effect ping --root ...` | Correctly blocked; exit 1; `operator STOP is active` |
| 9 | `aegis safe clear --root ...` | Pass; STOP inactive, revision 2 |
| 10 | `aegis effect ping --root ...` | Pass; `{"ok":true}` |
| 11 | `aegis serve --root ... --socket aegis-smoke-... --requests 1` + `aegis ipc status` | Pass over Windows named pipe; zero-Core status returned |

## Test-count breakdown

- `aegis-contracts`: 6 unit tests.
- `aegis-ipc`: 5 integration tests.
- `aegis-platform`: 6 integration tests.
- `aegis-safe`: 2 unit tests + 6 integration tests.
- `aegis-storage`: 5 unit tests.

**Total: 30 passed, 0 failed.**

## What the tests cover

- Versioned IPC envelope validation, protocol rejection, frame-size rejection, non-object params.
- Lifecycle vocabulary and roster policy (`1,2,3,6,7` approved; `4,5` prohibited).
- Platform/Safe distinct application markers and distinct DB files.
- Schema revision mismatch rejection, wrong application marker rejection, relative-root rejection.
- Platform zero-Core startup, empty Core registry, separate table ownership, persisted identity.
- Owner proof creation, verification, wrong-token rejection, DPAPI-protected blob on Windows.
- STOP set/clear persistence, owner authorization, audit-chain validity, audit revisioning.
- STOP blocking an effect at execution time, then clearing and re-enabling.
- IPC status, STOP set, blocked effect, STOP clear, allowed effect.
- IPC unauthorized owner token, unsupported protocol, malformed JSON, missing STOP reason.
- Platform source neutrality check (no `sofia` references in Platform source).

## Not verified

- **Live-verified:** not claimed.
- **Owner-accepted:** not claimed.
- **Linux build/runtime:** Unix `cfg` paths are implemented but were not cross-compiled or tested on Linux in this environment.
- **Encryption at rest:** deferred by Q5-A.
- **Installed service/daemon:** not created or tested.
- **Core databases, relationships, consent, memory, chat, cognition, voice, avatar, TUI:** out of Batch 1 scope.

## Safety confirmation

- No legacy Sofía database, asset, service, task, or reference-repository file was opened writable or modified.
- No Core was created or activated.
- No adult capability was implemented or enabled.
- No production cutover, Git commit, or push occurred.
