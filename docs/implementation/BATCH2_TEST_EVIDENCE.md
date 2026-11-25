# Batch 2 — Test and Evidence

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
| 3 | `cargo test --workspace` | **103 passed, 0 failed** |
| 4 | `cargo build -p aegis-cli` | Pass |
| 5 | `aegis init --root D:\Aegis\var\batch2-smoke` | Pass |
| 6 | `aegis core init <A> --number 1 --name "Sofía"` | Pass; separate DB, integrity verified |
| 7 | `aegis core init <B> --number 2 --name "Mack"` | Pass; separate DB, distinct instance UUID |
| 8 | `aegis core list` | Pass; two entries, `installed` |
| 9 | `aegis core verify <A>` / `<B>` | Pass; `verified` |
| 10 | Duplicate `core init <A>` | Correctly failed, exit 1 |
| 11 | `aegis core rename <A> --name "Sofía Ada Lyra"` | Pass; UUID unchanged, revision 2, manifest resealed |
| 12 | `aegis core deactivate <B> --reason "batch2 smoke"` | Pass; `suspended`; DB/manifest hashes unchanged |
| 13 | Append bytes to Core A DB, `core verify <A>` | Correctly failed `DigestMismatch`, exit 1 |
| 14 | `core verify <B>` after A tamper | Pass; one-Core failure isolated |
| 15 | `aegis status` | Pass; `core_count=2`, `zero_core=false` |
| 16 | Named-pipe `aegis serve` + `aegis ipc status` | Pass; two-Core status returned |
| 17 | Uppercase UUID `A9A0A604-...` fresh init + lowercase verify | Pass after remediation; canonical lowercase stored |

## Test-count breakdown

| Crate | Tests |
|---|---:|
| `aegis-contracts` | 15 |
| `aegis-core-identity` | 18 |
| `aegis-core-install` | 12 |
| `aegis-core-integrity` | 13 |
| `aegis-core-registry` | 9 |
| `aegis-core-storage` | 12 |
| `aegis-ipc` | 5 |
| `aegis-platform` | 6 |
| `aegis-safe` | 8 |
| `aegis-storage` | 5 |
| **Total** | **103** |

## Coverage

- Separate SQLite DB per Core, distinct database-instance UUIDs, no cross-Core leakage.
- Path traversal, foreign identifiers, Core-number policy, redirected components.
- Existing database collision, foreign ownership, ownership substitution, missing DB, partial state.
- Immutable UUID, mutable display name, identity revision, canonical UUID handling.
- Fresh initialization, duplicate init rejection, interrupted checkpoint refusal, failed-checkpoint recovery.
- Schema revision, integrity manifest success, tamper, missing/invalid manifest, missing DB, SQLite corruption, ownership mismatch, authorized reseal.
- Platform registry uniqueness, lifecycle history, deactivation preservation, zero-Core Platform.
- Owner authorization for init/rename/deactivate.
- CLI/IPC regression and two-Core runtime smoke.

## Not verified

- **Live-verified:** not claimed.
- **Owner-accepted:** not claimed.
- Linux build/runtime.
- Encryption at rest.
- Installed service/daemon.
- WAL/sidecar/concurrent integrity behavior.
- Uninstall-preserving-data/reactivation workflows.
- Core cognition, memory, chat, relationships, voice, avatar, TUI.

## Safety confirmation

- No legacy Sofía database, asset, service, task, or reference-repository file was opened writable or modified.
- No Core runtime, cognition, memory, chat, voice, avatar, or adult capability was implemented.
- A-04, B-01, B-06 remain provisional.
- No production cutover, commit, or push occurred.
