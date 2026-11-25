# Agent Ownership

## Standing ownership

| Agent | Ownership |
|---|---|
| Agent 1 / Platform Engineering | `aegis-platform`, `aegis-service`, `aegis-host`, host/Fleet, diagnostics integration |
| Agent 2 / Core Runtime and Identity | `aegis-core-*`, `aegis-core-sdk`, Core roster installation flows |
| Agent 3 / Modules and Integrations | `aegis-modules`, module SDK/runtime, functional modules |
| Agent 4 / UI, UX, Assets, Embodiment | `aegis-ui`, TUI, voice, avatar, assets, workspaces |
| Agent 5 / QA and Security | Independent review and acceptance tests only; may update review docs, not implementation code |
| Orchestrator | Shared contracts, workspace Cargo, installer/roster glue, CLI integration, conflict resolution, final test evidence |

## Work completed 2026-11-24 (orchestrator session)

| Area | Files | Verification |
|---|---|---|
| Approved Core roster catalog | `crates/aegis-roster` | 7 tests; CLI tests |
| Installer/upgrade/uninstall | `crates/aegis-installer` | 10 tests; runtime smoke |
| Host identity/enrollment | `crates/aegis-host` | 6 tests; CLI tests |
| CLI integration | `crates/aegis-cli/src/main.rs`, `crates/aegis-cli/tests/cli.rs` | 20 E2E tests |
| Registry read initialization fix | `crates/aegis-core-registry/src/lib.rs` | CLI zero-Core list test |
| Module package + process runtime | `crates/aegis-modules` (`manager.rs`, `runtime.rs`, `src/bin/`) | 9 runtime tests; module suite |
| Conversation persistence/engine/retrieval | `crates/aegis-conversation` | 10 tests + retrieval unit; CLI chat tests |
| Tool registry/execution bridge | `crates/aegis-tools` | 11 tests; CLI tool tests; conversation integration |
| Module-provided tools | `crates/aegis-modules` manifest/registry/tool_runner, `crates/aegis-tools` | 6 module-tool tests; manifest validation tests; real child processes |
| Embeddings + semantic retrieval | `crates/aegis-embeddings`, `crates/aegis-conversation/src/embedding_retrieval.rs` | 7 embedding tests; 2 retriever integration tests; mock Ollama HTTP |
| Social groups + private relationships | `crates/aegis-social`, CLI `social` | 8 tests; CLI membership/relationship flow |
| Approved Core profiles | `crates/aegis-core-profiles` | 9 tests; personality application/preservation |
| Fleet registry/placement | `crates/aegis-fleet`, CLI `fleet` | 9 tests; live local Ollama probe/route |
| GPU telemetry/admission/yield | `crates/aegis-fleet/src/gpu.rs`, `aegis-cognition` unload, CLI `fleet gpu` | Live RTX 3080 Ti telemetry, admission, cooperative unload |
| Live cognition/embeddings | `aegis-cognition`, `aegis-embeddings`, CLI | Live chat/streaming, 768-dim embeddings, semantic conversation+memory |
| Asset library + appearance/embodiment | `crates/aegis-embodiment` | 7 tests; CLI asset/appearance tests |
| Safe audit append | `crates/aegis-safe/src/store.rs` | `record_event` used by tool bridge |
| Completion documentation | `docs/completion/*`, `README.md` | Updated to verified state |

Rules: no concurrent edits to the same file; do not modify the legacy repository; no commit/push without explicit authorization.
