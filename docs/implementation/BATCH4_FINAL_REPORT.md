# Batch 4 — Final Report

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Branch:** `master` (baseline commit `2bd2513`)
**Scope:** Lab development only. No deployment, no production activation.

## Repository

| Item | Value |
|---|---|
| Baseline commit | `2bd2513` — 182 files, 354 tests passing |
| Modified tracked files | 11 |
| Untracked (non-ignored) files | 7 (3 source, 4 docs) |
| Ignored top-level entries | `target/`, `var/`, `.tmp/` |
| Recovery bundle | `.tmp/b4-recovery/tracked-changes.patch` (31,276 B) + 7 untracked copies + manifest |
| Committed in Batch 4 | No (owner approval required) |
| Secret scan (tracked + untracked) | No hits |
| Live data in Git | `git ls-files var` = 0 |
| Legacy repository | `D:\SofiaDev\SofiaAdaLyra` present, 47 entries, newest mtime 2026-11-22 (untouched) |

`.gitignore` was strengthened (no source excluded): added `*.log`, private-key
patterns (`*.key`, `*.pem`, `*.p12`, `*.pfx`, `id_rsa*`, `id_ed25519*`),
`.env`/`.env.*` (with `!.env.example`), and `*.local.{toml,json}`. No currently
required source file matches these patterns.

Recovery method: the working tree is authoritative; the patch plus copied
untracked files in `.tmp/b4-recovery/` allow reconstruction of the full
Batch 3 + Batch 4 delta against `2bd2513`. No destructive Git operation was run.

## Gaming-aware GPU yield (Venus, RTX 3080 Ti 12,288 MB)

**Game:** Phasmophobia (Steam app `739630`). Steam URL launch did not start the
game within 90 s; the game was started via its normal executable while the Steam
client was running. No game settings were changed.

| Measurement | Value |
|---|---|
| Detected process / PID | `phasmophobia`; main PID 7928 (+ bootstrap 28388) |
| Detection source | live `tasklist` (`detected=true`) |
| Idle GPU | 1,776 MB used, 3%, 60 °C |
| After model load | 8,691 MB used, 33%, 62 °C (`qwen3.5:9b`) |
| With game + model | 9,183 MB used, 7%, 58 °C (main menu) |
| Yield | model unloaded; free VRAM 3,091 → 9,674 MB; **reclaimed 6,583 MB**; floor met |
| Game during/after yield | `Phasmophobia` running, `Responding=True`; no crash observed |
| Hysteresis | game still detected on next sample; no reload/oscillation |
| Deferred work while game present | 2 jobs held (not released) |
| Game close → resumption | yield in second session reclaimed 6,916 MB; after 2 idle samples `resumed` released `resume-embed`, `resume-rag`; pending 0; 10 further idle samples showed no oscillation |
| Telemetry unavailable | `--nvidia-smi <missing>` ⇒ `available=false`, `admit=false`, action `telemetry_unavailable`, mode `unknown`; no unload |

**Limitations (explicit):**
- No FPS or frame-time telemetry source was available; none is reported.
- The game was observed in its main menu (utilization 7–14%); a heavy in-game
  gameplay session was not run.
- "Game remained operational" means the process stayed alive and responsive;
  no claim is made about absence of stutter or frame drops.
- Gaming-safe operation is **not** claimed merely from detection + unload.
- Global GPU policy was not changed; defaults (25% util, 2,048 MB free, 80 °C)
  were used.

## Fleet (Artemis / Venus)

**Artemis prerequisites (non-invasive):** NetBIOS name `ARTEMIS`, `WORKGROUP`;
open TCP 135/139/443/445/3389/5985 (WinRM); closed 22 (SSH), 80, 8080, 9090,
11434 (no Ollama). No remote execution was attempted.

**Deployment status: BLOCKED.** Installing a Fleet agent on Artemis requires
owner-authorized credentials/approval that were not provided. Per operating rule
9, no deployment was performed and no credentials were extracted or brute-forced.

| Result | Value |
|---|---|
| Venus identity handshake (separate processes, LAN) | `verified=true` (Batch 3; implementation unchanged) |
| Authentication failure (wrong secret) | rejected: "remote identity proof is invalid", exit 1 |
| Artemis registry entry | enrolled, `role=general`, `http://192.168.1.55:11434` |
| Artemis probe | `reachable=false` (timeout); excluded from placement |
| Real Artemis authenticated inference | **NOT ACHIEVED** (no agent/endpoint/credentials) |
| Venus LAN inference endpoint | probe/route/remote-inference verified in Batch 3; used only as the local LAN peer, not as Artemis |

Artemis is represented as a non-inference Platform/Core host and is excluded
from inference placement; it is **not** falsely advertised as an inference
endpoint.

## Cross-host resilience

| Test | Result |
|---|---|
| Provider timeout | blackhole LAN endpoint → `request timed out` (os 10060) at ~2,086 ms |
| Remote interruption during inference | slow server killed mid-request → `network error ... forcibly closed` (os 10054) |
| Authentication failure | HMAC proof rejected, exit 1 |
| Stale heartbeat | killed Core host → state `stale`, `heartbeat_fresh=false`, `heartbeat is stale` |
| Core DB isolation during Fleet failure | Sofía and Mack integrity `verified`; Sofía memory match 1, Mack oolong matches 0 |
| Deferred workload recovery | released on real-game close after idle hysteresis |
| Conversation persistence across restart | 2 messages read back by a fresh process |
| Automatic retries | none added (no duplicate non-idempotent operations) |

## Platform / Cores

Installed set preserved: #1 Sofía Ada Lyra, #2 Mack Fuller, #3 Nova Arden,
#6 Vera (installed). #4/#5 design-only; #7 unwritten.

- Diagnostics: 4 Cores, all integrity `verified`.
- Upgrade: `already_current` (no-op), version 0.1.0.
- Multi-Core startup: Sofía (pid 28956) and Nova (pid 29292) concurrently
  running with fresh heartbeats.
- Lifecycle: Mack reactivated; Sofía/Nova restored to `active` after tests.
- Sofía identity unchanged (`bbcce50a…`, number 1, "Sofía Ada Lyra").
- Per-Core asset list for Sofía is empty in this root (no assets imported); the
  legacy Sofía assets on disk were not modified.

## Code changes in Batch 4

- `.gitignore`: added log/key/credential/`.env`/local-override patterns (no
  required source excluded).
- `aegis-cli`: added `--nvidia-smi <path>` to `fleet gpu status|admission|supervise`
  as a diagnostic probe override so telemetry-unavailable behavior can be
  exercised without touching system files. Admission/yield policy is unchanged.
- Documentation: `BATCH4_FINAL_REPORT.md`, `BATCH4_REMAINING_WORK.md`,
  `BATCH4_QA_REVIEW.md`.
- No change to the GPU policy defaults, authentication scheme, integrity checks,
  or Core architecture.

## Quality

| Gate | Result | Log |
|---|---|---|
| `cargo fmt --all -- --check` | exit 0 | `.tmp/b4-fmt-check.log` |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 | `.tmp/b4-clippy.log` |
| `cargo build --workspace` | exit 0 | `.tmp/b4-build.log` |
| `cargo test --workspace` | **371 passed, 0 failed, 0 ignored** | `.tmp/b4-test-workspace.log` |
| Targeted Fleet | pass (23 unit + 9 integration) | `.tmp/b4-test-fleet.log` |
| Targeted Cognition/Ollama | 15 passed | `.tmp/b4-test-cognition.log` |
| Targeted Core crates | pass | `.tmp/b4-test-cores.log` |

No tests were removed, skipped, or weakened. The live-only cancellation test
still skips without `AEGIS_OLLAMA_LIVE=1` (pre-existing gating).

## Classification (independent statuses)

| Capability | Implemented | Tested | Integrated | Live-verified | Independently accepted |
|---|---|---|---|---|---|
| Gaming process detection | Yes | Yes | Yes | Yes (Phasmophobia PID 7928) | No |
| Gaming-aware admission/yield | Yes | Yes | Yes | Yes (real game yield) | No |
| Cooperative unload + VRAM recovery | Yes | Yes | Yes | Yes (6,583 MB) | No |
| Deferred resumption + hysteresis | Yes | Yes | Yes | Yes (real game close) | No |
| Telemetry-unavailable fail-safe | Yes | Yes | Yes | Yes (missing probe) | No |
| HMAC remote host identity | Yes | Yes | Yes | Yes (separate processes, LAN) | No |
| Artemis Fleet agent integration | No | No | No | No | No |
| Real Artemis authenticated inference | No | No | No | No | No |
| LAN Fleet probe/route/inference | Yes | Yes | Yes | Yes (Venus LAN peer) | No |
| Provider timeout / interruption handling | Yes | Yes | Yes | Yes | No |
| Stale heartbeat detection | Yes | Yes | Yes | Yes | No |
| Core DB / memory isolation under failure | Yes | Yes | Yes | Yes | No |
| Core set preservation | Yes | Yes | Yes | Yes | No |
| Voice / STT / TTS | No | No | No | No | No |
| Avatar rendering | Partial | Partial | No | No | No |
| Native GUI / OS services / Linux | No/Partial | Partial | No | No | No |
| Independent QA / owner acceptance | No | No | No | No | No |

## Verdict

- **Continued lab development: GO** (tree green, work recoverable).
- **Production readiness: NO-GO** (Artemis integration blocked; voice/STT/TTS,
  avatar rendering, OS services, Linux, cross-host Artifact Library incomplete;
  no owner acceptance).
- **Independent acceptance: NOT AVAILABLE.**
- Artemis deployment requires **owner authorization and credentials** before it
  can be attempted.
