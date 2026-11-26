# B. Repository Architecture Map (actual)

**Scope:** `D:\Aegis` at HEAD `2bd2513` + dirty working tree. 29 workspace
members, synchronous Rust (rusqlite/serde/sha2/uuid), no async runtime.

## Layered map

| Layer | Crate(s) | Key surfaces | Persistence |
|---|---|---|---|
| Contracts | `aegis-contracts` | identity/storage/lifecycle/roster DTOs, IPC, world, personal, integrity | none |
| Roster/profiles | `aegis-roster`, `aegis-core-profiles` | approved 7-Core catalog; personality/appearance/voice profiles | profiles write `memory.db` |
| Storage/security | `aegis-storage`, `aegis-core-storage`, `aegis-core-integrity`, `aegis-safe` | paths, SQLite guards, app markers, integrity manifests, owner proof/STOP/audit | `platform.db`, `safe.db`, `owner.proof`, `core.db`, `core.integrity.json`, init markers |
| Platform | `aegis-platform`, `aegis-ipc`, `aegis-host` | PlatformService/status/STOP, world/weather, named-pipe IPC, host identity/enrollment | `platform.db` tables; `run/service.json` |
| Core lifecycle | `aegis-core-identity`, `aegis-core-registry`, `aegis-core-install`, `aegis-core-runtime` | identity checkpoints, registry/lifecycle, install/`.corepkg`, supervised core host | `platform.db` registry/runtime; `run/<uuid>.heartbeat` |
| Core experience | `aegis-core-sdk` | CoreContext, memory/personality, personal emotion/continuity/activities/routines, event ledger | `memory.db`, `personal.db` |
| Cognition/conversation | `aegis-cognition`, `aegis-conversation`, `aegis-embeddings` | provider-neutral Ollama, chat/turn engine, retrieval, embeddings | `conversation.db`, `embeddings.db` |
| Embodiment | `aegis-embodiment` | assets/appearance, expressions/gestures, animation/scene/raster/sequence/lipsync, voice TTS/STT, remote artifact | `assets.db` + `assets/` files |
| Social | `aegis-social` | private groups, per-Core relationships/events, bounded awareness | `platform.db` groups; `social.db` |
| Modules | `aegis-modules`, `aegis-tools` | manifest/registry/manager/process runtime/sandbox probe; tool grants/approval/STOP/audit | `platform.db` module/tool tables; `modules/<id>` |
| UI | `aegis-ui` | loopback HTTP dashboard, ANSI TUI, summary JSON | none |
| CLI | `aegis-cli` | monolithic command dispatcher (init/install/core/module/chat/memory/asset/appearance/social/fleet/world/personal/voice/backup/...) | none |
| Ops | `aegis-diagnostics`, `aegis-backup`, `aegis-installer`, `aegis-service`, `aegis-fleet` | read-only diagnostics; v2 backup/restore; install/upgrade/preflight; service lifecycle; Fleet/GPU/queue | `platform.db`, backup dirs, `run/` |

## Persistence inventory

- Root: `platform.db` (identity, config, registry, runtime, world, fleet, host,
  social groups, modules, tools), `safe.db` (owner/STOP/audit), `owner.proof`.
- Per Core: `core.db`, `core.integrity.json`, `memory.db`, `conversation.db`,
  `assets.db` + `assets/**`, `social.db`, `embeddings.db`, `personal.db`,
  `init/<uuid>.initialization.marker`.
- Runtime: `run/<uuid>.heartbeat|stop`, `run/service.json`, `run/modules/<id>`,
  optional `run/status/*.json` (UI snapshots; no in-repo emitter).

## Key disconnects / duplicates found

1. **UI snapshot panels have no producer.** `aegis-ui` reads
   `run/status/{fleet,gpu,inference,service,world,personal}.json`; no crate
   writes them (only tests). Fleet/GPU/inference/service/world/personal are
   otherwise CLI-only.
2. **World distribution is caller-injected.** `WorldService` is real, but no
   IPC/background distribution; `aegis-ipc` exposes no world method.
3. **Social/wardrobe/fleet/host/installer are CLI-only.** `aegis-ui` lacks those
   crate dependencies; no GUI/TUI surfaces.
4. **Roster duplication/drift.** Approved numbers/names exist in `aegis-roster`,
   `aegis-core-profiles`, `aegis-core-install::package::FROZEN_CORE_DESIGNS`,
   `aegis-contracts::lifecycle`, and registry SQL checks.
5. **Two registry tables.** `platform_core` (legacy) and `core_registry`
   (authoritative) coexist; diagnostics/backup fall back to legacy.
6. **Three supervision implementations.** `aegis-service`, `aegis-core-runtime`,
   `aegis-modules::runtime` each implement process/heartbeat/termination.
7. **Two Ollama HTTP adapters** (`aegis-cognition`, `aegis-embeddings`) and two
   retrieval paths.
8. **Module packages excluded from backup and uninstall deletion.**
9. **UI reads Core DBs directly** (`CoreInstaller`, `ModuleRegistry`,
   `Diagnostics`) rather than an authenticated Platform API, contrary to the
   blueprint.
10. **Capability manifest overstates.** `CapabilityManifest::full` advertises 23
    capabilities to every Core, while the SDK context wires only a subset.

## Module ecosystem

Framework only: manifests/registry/manager/process runtime/tool bridge exist;
the only manifest is `examples/hello.manifest.json` (declares `wasm`, which the
runtime rejects) and two test binaries. **0/55 historical catalog modules.**

## UI/CLI reality

CLI is the real operational control plane. GUI is a loopback HTTP dashboard with
a narrow Core-lifecycle/STOP mutation set. TUI is a refresh-only snapshot
monitor. No native window, chat UI, social UI, wardrobe/object UI, or developer
workspace exists.
