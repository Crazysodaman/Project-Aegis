# Batch 3 — Test Evidence

**Date:** 2026-11-26
**Baseline:** 354 passed / 0 failed (commit `2bd2513`)
**Final:** 371 passed / 0 failed / 0 ignored

## Automated tests

| Suite | Command | Result |
|---|---|---|
| Workspace | `cargo test --workspace` | 371 passed, 0 failed |
| Fleet (unit, incl. new) | `cargo test -p aegis-fleet --lib` | 23 passed, 0 failed |
| Fleet (integration) | `cargo test -p aegis-fleet` | 9 passed, 0 failed |
| Cognition (Ollama) | `cargo test -p aegis-cognition --test ollama_mock` | 15 passed, 0 failed |

New coverage added in this batch:

- `gaming::tests` (5): name normalization, tasklist parsing, policy bounds,
  disabled policy, real live-process detection.
- `supervisor::tests` (6): gaming-signal yield at low utilization, unavailable
  gaming probe is not a detection, unload-failure reporting, deferred-workload
  release on resume, VRAM recovery measurement.
- `remote::tests` (6): valid/invalid proof, replay/host/callsign tampering,
  short-secret/input rejection, RFC 4231 HMAC vector, hex round-trip.
- `live_ollama_cancel_under_load_then_recovers` (1): live-only; skips without
  `AEGIS_OLLAMA_LIVE=1`.

## Live evidence (Venus unless noted)

| Area | Evidence file | Observed |
|---|---|---|
| Gaming process detection | `.tmp/b3-detect-steam.json` | `detected=true`, `steam` |
| Gaming-signal yield + recovery | `.tmp/b3-gpu-supervise.json` | `Yielded qwen3.5:9b`; 3,555→10,470 MB free; reclaimed 6,915 MB; floor met |
| Deferred resumption + hysteresis | `.tmp/b3-gpu-resume.json` | 6,920 MB reclaimed; `Resumed` released `embed-batch-1`, `rag-reindex`; pending 0 |
| Identity handshake over LAN | `.tmp/b3-id-verify.json`, `b3-id-server.out` | separate processes on `192.168.1.71:12001`; `verified=true` |
| LAN Fleet probe/route/inference | `.tmp/b3-fleet-probe2.json`, `b3-fleet-route.json`, `b3-fleet-infer.json` | reachable, model_count 1, route to LAN, `lan-ok` |
| Network interruption | `.tmp/b3-fleet-interrupt.json`, `b3-fleet-route-fail.txt` | unreachable (10061), `NoHealthyHost`, platform OK |
| Recovery | `.tmp/b3-fleet-recovered.json` | reachable again, routing recovered |
| Real Artemis probe | `.tmp/b3-artemis-probe.json` | `reachable=false` (11434 timed out), excluded from placement |
| Core install #1/#2/#3/#6 | `.tmp/b3-core1..6.json`, `b3-core-list.json` | integrity verified for all |
| Profiles | `.tmp/b3-profile1..6.json` | Sofía preserved; Mack/Nova applied; Vera external |
| Memory isolation | `.tmp/b3-mem-*.json` | Sofía finds oolong; Mack 0 matches, finds espresso |
| Lifecycle | `.tmp/b3-lc-deact.json`, `b3-lc-react.json`, `b3-lc-verify.json` | suspended→active, verified |
| Multi-Core startup | `.tmp/b3-rt-sofia.json`, `b3-rt-mack.json` | pids 28724 / 30368, fresh heartbeats |
| Roster prohibitions | `.tmp/b3-core4-attempt.txt`, `b3-core7-attempt.txt` | #4 prohibited; #7 needs name |
| Service restart | `b3-svc-start1.json`, `b3-svc-status4.json`, `b3-svc-stop.json` | start→stop→new pid 29716 running |
| Cancellation under load | `.tmp/b3-live-cancel.log` | canceled after ≥3 chunks, follow-up chat ok |
| Conversation persistence | `.tmp/b3-chat-list.json`, `b3-chat-messages.json` | fresh process read 2 persisted messages |
| Separate-process QA | `.tmp/b3-qa-process.log` | `QA_SIGNALS_GREEN` |

## Not verified

- Live in-game gaming workload (only launcher + process fixture).
- Authenticated remote inference on real Artemis.
- Voice/STT/TTS, avatar rendering, OS services, Linux runtime, native GUI,
  cross-host Artifact Library.

## Notes

- `.tmp/` and `var/` are git-ignored; evidence is retained locally and not
  committed. No secrets, owner proofs, or live databases are tracked.
