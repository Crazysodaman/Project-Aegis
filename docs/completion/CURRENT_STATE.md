# Current State

**Date:** 2026-11-24 (Wave 1d + conversation + embodiment batches)  
**Repository:** `D:\Aegis`  
**Branch:** `master` (unborn HEAD)  
**Commits:** none  
**Working tree:** all project files untracked; no commit or push performed

## Verified baseline

- `cargo fmt --all -- --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — **350 passed, 0 failed**.
- CLI E2E tests - 23 passed (real binary under `target\debug\aegis.exe`).
- Module runtime integration — 9 passed with real child processes.
- Runtime smoke on `D:\Aegis\var\smoke-2026-11-24` — pass:
  - `aegis install --headless`
  - `aegis roster show 1`
  - `aegis core install <uuid> --roster 1`
  - `aegis core verify <uuid>`
  - `aegis diagnostics` (1 Core, verified)
  - `aegis backup --to <sibling>` (6 files, verified)
  - `aegis upgrade` (already current)
  - `aegis uninstall` (preserve; data retained)
  - Host enrollment exercised through CLI tests.

## Implemented surfaces

- Platform foundation, Safe owner proof/STOP/audit, JSON-RPC IPC, background service process, diagnostics.
- Host identity and explicit owner-authorized enrollment/revocation with append-only history.
- Installer: versioned install record, pre-upgrade verified backup, downgrade refusal, preserve/delete uninstall with callsign confirmation.
- Approved Core roster catalog: #1 Sofía Ada Lyra, #2 Mack Fuller, #3 Nova Arden, #6 Vera/S-039 installable; #4/#5 prohibited; #7 blank identity requiring an owner-supplied name.
- Per-Core identity/storage/integrity/registry/installation and supervised runtime (real child process, heartbeat, stop sentinel).
- Core SDK: capabilities, personality/growth, per-Core memory lifecycle.
- Module framework: package materialization, declarative registry, owner-authorized lifecycle, and supervised process execution (heartbeat, crash detection, forced kill, failure isolation).
- Conversation layer: per-Core `conversation.db` with sessions/messages/events/export; provider-independent turn assembly with tokenized private-memory retrieval, descriptive personality context, streaming events, and tool-call storage as data.
- Embodiment interfaces: per-Core asset library with SHA-256 integrity, plus appearance/theme/avatar/expression/gesture/wardrobe/voice-profile configuration.
- Tool execution bridge: grant/default-deny registry, owner approval for sensitive tools, STOP enforcement, argument validation, Safe audit, conversation integration.
- Memory retrieval is a pluggable interface with per-call method reporting; keyword fallback, a deterministic local feature-hash embedding provider, and a real Ollama embeddings adapter (mock-tested, live unverified).
- Module-provided tools: manifest declarations are exposed as Platform-namespaced tools, executed as one-shot module child processes only when enabled, through the central grant/approval/STOP/audit bridge.
- Social runtime: Platform-owned private groups with explicit membership enforcement and immediate revocation; per-Core private relationships with revisioned history and ownership-bound storage.
- Live verified: Ollama 0.40.17 chat/streaming (`qwen3.5:9b`, `think:false`), real 768-dim `nomic-embed-text` embeddings, semantic conversation+memory retrieval, local Fleet probe/routing, RTX 3080 Ti telemetry, and cooperative GPU yield (9.2 GB to 2.3 GB VRAM) with admission recovery.
- GPU telemetry/admission: nvidia-smi probe, fail-closed busy/low-VRAM/hot/missing-GPU policy; live telemetry verified; gaming-load yield not live-tested.
- Approved Core profiles: data-driven Mack/Nova personality application, Sof�a preservation, Vera external-spec reference, blank Core #7, and prohibited twins.
- Fleet registry: owner-gated host add/enroll/revoke/remove, Ollama endpoint health/model discovery, placement/model routing, and adaptive-policy flag.
- Backup/verify/restore.
- Cognition: provider-independent interface and real Ollama HTTP adapter (mock-server tests only).
- UI: loopback HTTP dashboard and TUI; dashboard tests present.
- CLI command surface including install/upgrade/uninstall, roster, host, core, module, diagnostics, memory, provider, backup, serve, gui, tui.

## Not implemented / not verified

- Avatar rendering, audio voice synthesis/playback, Core-to-Core social awareness/trust semantics, group UX in GUI/TUI, and live Fleet/GPU verification (stores/interfaces exist).
- Functional module roster (inference, voice, assets, social, operational) and OS-level module sandboxing.
- Fleet/host agent, remote placement, migration.
- OS-level service installation/removal; reboot recovery.
- Linux build/runtime.
- Live Ollama/GPU inference.
- Native GUI window.
- Adult capability (remains fail-closed; A-04 provisional).
- Owner acceptance.

## Live verification

See `LIVE_VERIFICATION_2026-11-25.md`. Embeddings are now live-verified after resolving the Ollama key-path issue. Remote Venus traffic, automatic gaming-load detection, cancellation, voice/avatar, Linux, services, GUI, and independent QA remain outstanding.

## Next concrete action

Verify the real Ollama chat/embedding adapters and Fleet/Venus GPU behavior on configured hardware, then complete social UX, voice/avatar rendering, GUI, OS services, Linux, and Artifact Library. Core profiles, Fleet registry, social groups, module tools, and embedding retrieval foundations are implemented; live provider/hardware verification is outstanding.

## Resume commands

```powershell
Set-Location D:\Aegis
git status
cargo test --workspace
cargo run -p aegis-cli -- roster list
cargo run -p aegis-cli -- module runtime-list --root D:\Aegis\var\smoke-2026-11-24
```
