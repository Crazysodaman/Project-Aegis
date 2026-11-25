# Project Aegis — Greenfield Rust Workspace

This repository is the greenfield implementation target for Project Aegis. It is governed by:

- `D:\SofiaDev\SofiaAdaLyra\docs\specs\AEGIS_MASTER_SPECIFICATION_v0.3.0.md`
- `D:\SofiaDev\SofiaAdaLyra\docs\aegis-discovery-v0.2.0\06_OWNER_DECISIONS_LOCKED.md`
- `D:\SofiaDev\SofiaAdaLyra\docs\specs\SPEC-03_CORE_IDENTITY_AND_CONSTITUTION.md`
- `D:\SofiaDev\SofiaAdaLyra\docs\specs\SPEC-12_CORE_DATA_OWNERSHIP_PRIVACY_AND_RETENTION.md`
- `D:\SofiaDev\SofiaAdaLyra\docs\specs\SPEC-13_DATABASE_AND_ASSET_STORAGE.md`
- `D:\SofiaDev\SofiaAdaLyra\docs\aegis-discovery-v0.2.0\04_REBUILD_DEPENDENCY_GRAPH.md`

The reference repository is strictly read-only. No legacy Sofía-coupled code, data, assets, services, or databases are imported or modified.

## Implemented today (local development)

- Core-neutral Platform foundation, zero-Core startup, status, diagnostics, JSON-RPC IPC.
- Safe owner proof, STOP, and append-only security audit.
- Background Platform service process with start/stop/status and reachability checks.
- Host identity with explicit owner-authorized enrollment/revocation and history.
- Installer with versioned installation record, pre-upgrade verified backup, downgrade refusal, and preserve/delete uninstall.
- Approved Core roster catalog: #1 Sofía Ada Lyra, #2 Mack Fuller, #3 Nova Arden, #6 Vera/S-039 installable; #4/#5 prohibited; #7 blank and owner-named.
- Per-Core identity, separate SQLite database, integrity manifest, registry/lifecycle history, installation/recovery.
- Supervised Core runtime: real child process, bound heartbeat, stale detection, sentinel stop, forced-kill timeout.
- Core SDK: capabilities, personality/growth, per-Core memory lifecycle.
- Module framework with strict manifests, package materialization, owner-authorized lifecycle, and supervised process execution (heartbeat, crash detection, forced kill, failure isolation).
- Per-Core conversation persistence and a provider-independent interaction engine integrating private memory, descriptive personality, and streaming events.
- Per-Core embodiment interfaces: verified asset library plus appearance/theme/avatar/expression/gesture/wardrobe/voice-profile configuration.
- Authorized tool execution bridge with default-deny grants, owner approval for sensitive tools, STOP enforcement, argument validation, and Safe audit; module-provided tools execute through the same bridge.
- Provider-independent memory retrieval with per-call method reporting: keyword fallback, local feature-hash embeddings, and a real Ollama `/api/embeddings` adapter (live unverified).
- Social runtime: Platform-owned private groups with explicit membership enforcement, plus per-Core private relationship records with revisioned history.
- Approved Core profile data (Sof�a preservation, Mack/Nova personality application, Vera external-spec reference, blank #7, prohibited twins), an explicit-trust Fleet host registry with health probes/placement/model routing, and nvidia-smi GPU telemetry with fail-closed admission policy.
- Live-verified locally: Ollama 0.40.17 chat/streaming (`qwen3.5:9b`, `think:false`), real 768-dim `nomic-embed-text` embeddings, semantic conversation+memory retrieval, Fleet probe/routing, RTX 3080 Ti telemetry/admission, and cooperative GPU yield (9.2 GB to 2.3 GB VRAM). Automatic gaming-load detection remains unverified.
- Backup, verify, and restore.
- Provider-independent cognition with a real Ollama HTTP adapter (mock-server tests only; no live model verified).
- Loopback HTTP dashboard and TUI.

## Not implemented

Relationships/social/group conversations, avatar rendering, audio voice synthesis/playback, functional module roster, OS-level module sandboxing, Fleet/placement, OS service installation, native GUI, Linux verification, live model inference, and adult capability (fail-closed). Cores #4/#5 remain prohibited; Core #7 has no invented identity. A-04, B-01, and B-06 remain provisional.

## Verification

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Current baseline: **350 passed, 0 failed** on Windows. See `docs/completion/` for the ledger and evidence. No Owner acceptance is claimed.

## Command surface

```powershell
aegis install --root <path> [--headless]
aegis upgrade --root <path> [--backup-to <new-directory>]
aegis uninstall --root <path> [--delete-data --confirm <callsign> --backup-to <new-directory>]

aegis init --root <path> [--print-owner-token]
aegis status --root <path>
aegis diagnostics --root <path>

aegis host status|enroll|revoke|history --root <path>

aegis roster list
aegis roster show <core-number>

aegis core install <core-id> --root <path> --roster <n> [--name <name>]
aegis core init <core-id> --root <path> --number <n> --name <name>
aegis core list|inspect|verify|rename|deactivate|reactivate|uninstall-preserve|start|stop|runtime-status ...

aegis module list|install|install-package|enable|disable|uninstall|inspect|verify ...
aegis module tools <module-id> --root <path>
aegis module start|stop|restart|runtime-status|runtime-list ...
aegis memory search|create|export|personality <core-id> --root <path> ...
aegis chat create|list|messages|send|export|archive|delete <core-id> --root <path> ...
aegis asset import|list|show|read|delete|export <core-id> ... --root <path>
aegis appearance show|set-avatar|set-expression|set-gesture|set-wardrobe|set-theme|set-voice ... <core-id> --root <path>
aegis tool list|grant|revoke|execute ... --root <path>
aegis social group|relationship ... --root <path>
aegis core profiles | core profile <n> | core apply-profile <core-id> --root <path>
aegis fleet host|placement|route|policy|gpu ... --root <path>
aegis provider health|list|chat|embed ...
aegis backup --root <path> --to <new-directory>
aegis verify-backup --path <backup-directory>
aegis restore --from <backup-directory> --root <target-root> [--allow-existing]
aegis serve --root <path> --socket <name> [--requests <n>]
aegis gui --root <path> [--port <n>] [--allow-mutations]
aegis tui --root <path> [--refresh-ms <n>] [--once]
```

`init` prints a development owner token only with `--print-owner-token`. Do not treat it as production owner proof.
