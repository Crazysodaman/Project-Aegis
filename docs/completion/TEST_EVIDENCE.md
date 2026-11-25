# Test Evidence

## Latest baseline (2026-11-24)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --workspace` | **350 passed, 0 failed** |
| `cargo build --workspace` | Pass |

## Test counts

| Area | Tests |
|---|---|
| contracts | 15 |
| core identity | 19 |
| core install | 12 |
| core integrity | 16 |
| core registry | 19 |
| core runtime | 9 |
| core storage | 13 |
| core SDK | 14 |
| core profiles | 9 |
| roster | 7 |
| installer | 10 |
| host identity/enrollment | 6 |
| CLI end-to-end | 23 |
| modules (unit + registry) | 21 |
| module process runtime | 9 |
| module-provided tools | 6 |
| conversation engine/persistence | 13 |
| conversation retrieval (unit) | 1 |
| embeddings | 9 |
| social groups/relationships | 8 |
| fleet registry/placement | 9 |
| GPU telemetry/policy (unit) | 3 |
| tool registry/execution | 11 |
| diagnostics | 11 |
| backup/restore | 12 |
| cognition (Ollama mock server) | 14 |
| IPC | 5 |
| Platform | 6 |
| Safe | 8 |
| storage | 5 |
| service | 6 |
| UI dashboard | 14 |
| embodiment asset/appearance | 7 |
| **Total** | **350** |

## New coverage in this batch

- `aegis-modules`: manifest tool-declaration validation (valid, duplicate, invalid name, zero timeout, non-object schema); tool bindings from installed modules including disabled availability; one-shot module tool runner.
- `aegis-tools` module tools (6): listing and default-deny; grant then execute with real child process; disabled module denial; STOP and revocation denial; sensitive approval; failure/timeout/invalid-argument containment with Platform survival; caller-Core context isolation and audit integrity.
- `aegis-embeddings` (9): deterministic local feature-hash vectors and lexical similarity; cosine edge cases; per-Core cache ownership/cross-Core rejection; content invalidation; prune/clear; real Ollama `/api/embeddings` HTTP parsing including chunked transfer encoding; HTTP errors; URL rejection; live 768-dim embedding opt-in test.
- `aegis-conversation` (+2): embedding-backed ranking with reported method and cache population; embedding-provider failure falls back to keyword with an explicit `keyword(fallback: ...)` method.
- `aegis-core-profiles` (9): catalog/roster consistency; prohibited and blank constraints; Mack/Nova personality generation and persistence; Sofia existing-identity preservation; Vera external-spec non-invention; per-Core isolation; display-name resolution.
- `aegis-fleet` (9 + 3 GPU unit): owner-gated add/enroll/revoke/remove; untrusted default; mock Ollama health/model discovery; placement and model routing; unreachable-host reporting with Platform survival; policy persistence; endpoint validation; GPU telemetry parsing; fail-closed admission for busy/low-VRAM/hot/missing GPU; live GPU opt-in test.
- `aegis-social` (8): group create/add/remove/post/message attribution; membership enforcement and immediate revocation; non-member list/read/post denial; owner administrative access; duplicate/missing member errors; persistence; per-Core relationship create/update/revision/events/delete; Core isolation and cross-Core copy rejection; CLI group/relationship flow.

## Prior coverage retained

- Roster/installer/host/CLI, module lifecycle/runtime, conversation persistence and tool bridge, built-in tool grants/approval/STOP/audit, diagnostics, backup, cognition mock server, IPC, Platform/Safe/storage/service, UI dashboard, embodiment.

## Runtime smoke (2026-11-24)

Disposable root `D:\Aegis\var\smoke-2026-11-24`: install -> roster install -> verify -> diagnostics -> backup -> upgrade -> uninstall-preserve; host enrollment and module runtime exercised through CLI/integration tests.

## Live verification (2026-11-25)

See `LIVE_VERIFICATION_2026-11-25.md`. Verified live: Ollama 0.40.17 chat/streaming (`qwen3.5:9b`, `think:false`), real 768-dim `nomic-embed-text` embeddings, semantic retrieval (`embedding:ollama:nomic-embed-text`, `retrieval_semantic=true`), local Fleet enrollment/probe/routing, RTX 3080 Ti telemetry/admission, and cooperative model-unload yield (9.2 GB to 2.3 GB VRAM). Automatic gaming-load detection, remote Venus traffic, cancellation, voice/avatar, Linux, services, GUI, and independent QA remain unverified.

## Not executed

- Linux build/runtime and OS service installation.
- Automatic gaming-load detection/preemption and remote Venus (Artemis?Venus) traffic.
- Inference cancellation under live load.
- Voice playback, STT/TTS, avatar rendering, native GUI.
- Independent QA and Owner acceptance.

Live provider/hardware evidence is limited to the local Windows host, Ollama 0.40.17, `qwen3.5:9b`, `nomic-embed-text`, and the local RTX 3080 Ti. No Owner acceptance is claimed.
