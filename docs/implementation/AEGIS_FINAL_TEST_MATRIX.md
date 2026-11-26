# AEGIS FINAL — Test Matrix

**HEAD:** `2bd2513` · Workspace total: **942 passed, 0 failed, 1 ignored**

| Agent | Scope | Package-scoped result |
|---|---|---|
| 01 | Platform lifecycle | 38 platform (3 new lifecycle) |
| 02 | Fleet eligibility/reconnect | 119 fleet (3 new reconnect) |
| 03 | GPU queue/placement | 128 fleet (queue/gate) |
| 04 | World refresh/distribution | 68 contracts+platform (6 new) |
| 05 | Expressed emotion/relationship | contracts 30; core-sdk 33-integration |
| 06 | Memory consolidation/session summary | conversation 30 (7 new); core-sdk 33 |
| 07 | Event→emotion idempotent pipeline | 67 core-sdk+conversation (4 new) |
| 08 | SAPI voice style | embodiment 115 (4 new; 1 ignored live LAN) |
| 09 | Avatar state/sequence | embodiment 123 (7+1 new) |
| 10 | Wardrobe hard constraints | embodiment 128 (13 presentation) |
| 11 | GUI world/personal | UI 72 (6 new GUI) |
| 12 | CLI/TUI packaging/voice | CLI 23; UI unit 22 + GUI 37 + TUI 13 |
| 13 | Social events/awareness | social 23 (6 new) |
| 14 | Module dependency/version | modules 57 (6 new) |
| 15 | `.corepkg` packaging | core-install 34 (9 new) |
| 16 | Security adversarial | safe 21, roster 17, install 25, backup 23 |
| 17 | Live-safe backup | backup 29 (6 new) |
| 18 | Installer preflight | installer 20 + service 65 |
| 19 | Reliability/concurrency | 17 new across 5 packages |
| 20 | Independent review | repeated critical suites; 942/0/1 |

## Live / runtime evidence

| Check | Result |
|---|---|
| Full workspace | 942 passed, 0 failed, 1 ignored |
| Ignored live artifact test | passed (131,072 B, SHA verified) |
| SAPI TTS/STT | 184,334 B WAV; transcript verified |
| Avatar sequence | 18 frames, ~1.1 MB, differing SHAs |
| Weather | Open-Meteo live observation |
| World/chat continuity | live on disposable root |
| Backup/restore | live-safe snapshot + restore verified |
| `.corepkg` | validated install tests |
| CLI smoke | install/world/core/chat/personal/package/voice/backup |

## Unverified

Artemis two-host, Debian/systemd, native GUI, microphone STT/playback, GPU avatar
renderer, gameplay FPS, long-run soak, independent external QA, owner acceptance.
