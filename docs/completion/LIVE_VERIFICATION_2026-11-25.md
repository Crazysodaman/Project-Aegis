# Live Verification Evidence — 2026-11-25

Environment: Windows development host with an NVIDIA GeForce RTX 3080 Ti (12,288 MB),
local Ollama 0.40.17 at `http://127.0.0.1:11434`, Aegis root
`D:\Aegis\var\live-2026-11-25` (disposable).

No credentials or secrets are recorded here.

## Verified live

| Area | Evidence | Status |
|---|---|---|
| Ollama health/model discovery | `/api/version` = 0.40.17; six installed models discovered via `/api/tags`; Fleet probe returned `reachable=true`, `model_count=6`, `latency_ms=50` | Live provider verified |
| Real inference | `aegis provider chat --model qwen3.5:9b --prompt "Reply with exactly: live-ok"` → content `live-ok`, `done=true`, 160 eval tokens, 13.5 s | Live provider verified |
| Real streaming | `aegis provider chat --model qwen3.5:9b --prompt "Reply with exactly: stream-ok" --stream` → 264 content chunks, aggregated `stream-ok` | Live provider verified |
| Non-thinking control | qwen3.5 required `think:false` to return visible content; the new `--no-think` path was used for conversation inference | Live provider verified |
| Conversation + private memory | Created memory "The owner prefers oolong tea in the morning."; `chat send --retrieval local-hash --no-think` returned exactly that fact with `retrieval_method=embedding:local-hash:feature-hash-v1`, `retrieval_semantic=false`, one memory snippet, persisted user/assistant messages | Live provider + runtime verified |
| Fleet enrollment | Host `Venus` added untrusted, enrolled with owner proof, probed successfully against real Ollama | Live host verified (local host) |
| Fleet model routing | `fleet route --model qwen3.5:9b` selected `http://127.0.0.1:11434` (priority 90) after live model discovery | Live provider verified |
| GPU telemetry | `nvidia-smi` probe: `NVIDIA GeForce RTX 3080 Ti`, utilization 7%, 10,970/12,288 MB used, 56 °C, `available=true` | Live hardware verified |
| GPU admission | Admission denied with reason `vram_low` because free VRAM (~1,318 MB) was below the 2,048 MB policy floor | Live hardware + policy verified |
| Live embeddings | Installed `nomic-embed-text` using the existing trusted Ollama key; 768-dimensional vectors; related-text cosine 0.858 vs unrelated 0.499 | Live provider verified |
| Semantic retrieval | `chat send --retrieval ollama --embedding-model nomic-embed-text` returned `retrieval_method=embedding:ollama:nomic-embed-text`, `retrieval_semantic=true`, and the correct private memory | Live provider + runtime verified |
| GPU yield | `fleet gpu running` listed `qwen3.5:9b` and `nomic-embed-text`; `fleet gpu yield --model qwen3.5:9b` reduced VRAM from 9,248 MB to 2,325 MB; admission recovered to `admit=true` | Live hardware verified (cooperative unload) |

## Embedding blocker resolution

Root cause: the Ollama data/key directory is `D:\ollama`, while the CLI expected
`C:\Users\kenny\.ollama\id_ed25519`. The existing trusted key was copied to the
expected profile path (no signature verification was bypassed), after which
`ollama pull nomic-embed-text` succeeded. The embedding HTTP adapter also needed
chunked transfer-encoding decoding; this was fixed and covered by a regression test.

## Not verified / blocked

| Area | Reason |
|---|---|
| Live gaming-load yield | No gaming workload was launched. Cooperative model unload was live-verified, and admission is unit-tested with synthetic telemetry, but automatic gaming-load detection/preemption was not exercised. |
| Remote Venus↔Artemis traffic | The verified Ollama/GPU host is this local Windows host. No separate Artemis or remote Venus host was contacted. |
| Inference cancellation | Not live-tested. HTTP checks and cancellation logic are unit/mock-tested. |
| Voice/avatar | Not implemented; no audio or renderer provider verified. |
| Linux / OS services / native GUI | Not verified in this environment. |

## Caveats

- One installed model (`freehuntx/qwen3-coder:14b`) crashed the Ollama CUDA backend with a stack overrun; this is a model/backend failure, not an Aegis client success. Live verification was scoped to `qwen3.5:9b`.
- GPU metrics are point-in-time samples and may change. No GPU metric is invented when `nvidia-smi` is unavailable.
- Evidence is live for this host and these models only. It does not establish production readiness or owner acceptance.
