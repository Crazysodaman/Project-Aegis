# Batch 3 — Implementation Log

**Date:** 2026-11-26 (UTC evidence timestamps)
**Repository:** `D:\Aegis`
**Baseline:** commit `2bd2513` — 354 tests passing, 0 failed
**Post-change:** 371 tests passing, 0 failed

This batch continued from the reported 350-test baseline. Completed Ollama,
embeddings, semantic retrieval, Fleet registry, GPU telemetry, admission, and
cooperative-yield code was **not** rebuilt; it was extended.

## 1. Repository preservation

- Baseline commit `2bd2513` captured the entire workspace *before* any change so
  every modification is recoverable with `git diff`/`git checkout`.
- `git status` at preservation time: unborn `master`, all files untracked; 182
  files staged into the baseline commit.
- `.gitignore` excludes `/target/`, `/var/`, `.tmp/`, `*.db`/`*.sqlite` (and
  WAL/SHM), and `owner.proof`. Live databases and owner proofs are **not**
  tracked. `git ls-files var` = 0.
- Secret scan of tracked source/config/docs (`sk-…`, `AKIA…`, private-key
  headers) returned no hits in a separate QA process.
- `D:\SofiaDev\SofiaAdaLyra` exists and was not modified (47 entries, read-only
  reference preserved). No legacy repository write was performed.

## 2. Automatic GPU yield (extended, live-verified)

New `aegis-fleet` modules and supervisor changes:

- `gaming.rs` — real OS process-list gaming-workload detection.
  - Windows: `tasklist /FO CSV /NH`; Unix: `ps -eo comm=`.
  - Opt-in allow-list of executable stems (`GamingPolicy`), normalized
    (basename, lowercase, `.exe` stripped).
  - Unreadable process list ⇒ `available = false` with an error; never invents a
    detection.
  - A failed gaming probe is informational and does **not** by itself deny
    admission (GPU telemetry remains the fail-closed signal).
- `gpu.rs` — `evaluate_admission_with_gaming`; a detected gaming process denies
  background inference when `gaming_yield` is enabled, even at low GPU
  utilization (menu/loading/paused game).
- `supervisor.rs`:
  - `step_full(telemetry, gaming, running_models, unload, reprobe)` with the
    original `step` delegating unchanged.
  - Hysteresis retained: `busy_samples` before unload, `idle_samples` before
    resume, unload cooldown to avoid oscillation.
  - Cooperative unload via Ollama `keep_alive: 0` (existing `unload_model`).
  - Resource recovery: post-unload re-probe records before/after free VRAM,
    reclaimed MB, and whether the VRAM floor is met.
  - Workload resumption: `defer(id)` queues background work; on a resume
    transition the report releases `resumed_workloads` and clears the queue.
  - Failure handling: unload error is reported as `UnloadFailed` and yield is
    **not** claimed.
  - Telemetry-unavailable: mode `Unknown`, no unload attempted, admission false.
- CLI: `fleet gpu detect-gaming --game-process <name>...`; `fleet gpu supervise`
  now accepts repeatable `--game-process` and repeatable `--defer <id>`, probes
  the gaming signal each sample, and re-probes GPU state after unload.

Live results on Venus (RTX 3080 Ti, 12,288 MB):

| Check | Result |
|---|---|
| Real process detection | `--game-process steam` → `detected=true`, matched `steam` (live `tasklist`) |
| Gaming-signal yield at low util | 3% utilization, Steam detected → `Yielded qwen3.5:9b` |
| Cooperative resource recovery | free VRAM 3,555 MB → 10,470 MB; `reclaimed_mb=6915`; `meets_floor=true` |
| Hysteresis / no oscillation | Deferred work stayed pending while the gaming process remained |
| Live resumption | Controllable process fixture; yield reclaimed 6,920 MB; after 2 idle samples `Resumed`, released `embed-batch-1`, `rag-reindex`, pending 0 |

**Gaming-safe operation is NOT claimed.** No actual game was launched. The
detections used the real Steam launcher and a controllable real OS process
fixture, with real GPU VRAM effects. A live in-game run remains outstanding.

## 3. Remote Fleet (partially live-verified)

- `remote.rs` — authenticated remote host identity.
  - Fresh nonce challenge; HMAC-SHA256 proof over a domain-separated message
    bound to nonce + host UUID + callsign.
  - Constant-time proof comparison; shared secret ≥ 16 bytes.
  - Constant `MIN_SECRET_BYTES`, canonical response schema.
  - RFC 4231 HMAC test vector covered.
- `src/bin/aegis-fleet-identity.rs` — separate-process `serve`/`verify` agent;
  secret read from `AEGIS_REMOTE_SECRET` (never argv/logs).
- Live identity handshake across two processes over the LAN interface
  `192.168.1.71:12001`: `verified=true`, callsign `Venus`.
- LAN Fleet endpoint (mock Ollama bound to `192.168.1.71:11999`):
  - Probe: `reachable=true`, `model_count=1`, latency 3–4 ms.
  - Routing selected the LAN endpoint.
  - Remote inference returned content `lan-ok`.
  - Interruption (server stopped): `reachable=false`, OS 10061 error, routing
    refused with `NoHealthyHost`; platform status remained OK.
  - Recovery (server restarted): `reachable=true`, routing recovered.
- Real Artemis (`Artemis.local` = `192.168.1.55`) is reachable (ICMP, RDP 3389,
  SMB 445) but exposes no Ollama/Aegis endpoint (port 11434 timed out) and no
  remote-execution credentials were used. Real Artemis probe correctly returned
  `reachable=false` and Artemis was excluded from placement.

**Not claimed:** authenticated remote *inference* on Artemis. No agent runs on
Artemis and it exposes no inference endpoint; the live remote inference used a
LAN-bound endpoint on Venus.

## 4. Platform and Core deployment (audited, live-verified)

- Roster/profiles validated: #1 Sofía, #2 Mack, #3 Nova, #6 Vera installable;
  #4/#5 prohibited; #7 blank.
- Installed fresh Cores #1, #2, #3, #6; each integrity `verified`.
- Profiles: Sofía preserved (no overwrite), Mack/Nova applied, Vera owned by the
  approved external specification.
- Separate databases confirmed: `platform.db`, `safe.db`, and per-Core
  `core.db` / `memory.db` / `conversation.db`.
- Private memory isolation: Sofía found its oolong fact; Mack returned 0 matches
  and found only its own espresso fact.
- Lifecycle: Nova `active → suspended → active`; integrity still verified.
- Multi-Core startup: Sofía (pid 28724) and Mack (pid 30368) ran concurrently
  with fresh heartbeats.
- #4 install refused (`prohibited`); #7 install refused without owner name.
- Installer `upgrade` returned `already_current`; diagnostics reported 4 Cores,
  all integrity-verified.
- Core #7 remains unwritten; #4/#5 remain design-only. Sofía's legacy identity
  assets in `D:\SofiaDev\SofiaAdaLyra` were preserved untouched.

## 5. Live resilience (live-verified)

- Inference cancellation under load: long live streaming generation
  (`qwen3.5:9b`, 2048-token budget) cancelled after ≥3 chunks / ≤2 s → returned
  `Cancelled`; a follow-up live chat succeeded (provider recovered). 10.49 s.
- Provider failure: unreachable/refused endpoint produced a typed network error;
  routing refused cleanly.
- Service restart: `service start` → running (socket reachable), `service stop`
  → stopped, restart produced a new PID 29716 running/reachable.
- Conversation persistence and recovery: live turn persisted (user + assistant
  messages); a fresh process read the same session and messages. DBs survive
  process exit.

## 6. Remaining implementation status (honest)

Existing surfaces remain implemented and tested: social runtime (groups +
private relationships), process-based module execution, loopback GUI dashboard,
TUI, per-Core asset/Artifact library with SHA-256 integrity.

Still **not implemented / not live-verified** in this batch:

- Voice synthesis / STT / TTS playback (voice-profile configuration only).
- Avatar rendering (appearance/asset bindings only; no renderer).
- OS service registration (Windows SCM / systemd); reboot recovery.
- Linux build/runtime verification.
- Cross-host Artifact Library placement (e.g. on Artemis).
- Native GUI window; group UX in GUI/TUI.
- Adult capability (remains fail-closed).

These are not claimed as complete. No architectural expansion was added for them.

## Quality gates (actual)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (exit 0) |
| `cargo build --workspace` | pass (exit 0) |
| `cargo test --workspace` | **371 passed, 0 failed, 0 ignored** |

Logs: `D:\Aegis\.tmp\b3-fmt-check.log`, `b3-clippy2.log`, `b3-build.log`,
`b3-test-workspace.log`, plus live evidence `b3-*.json`/`b3-*.log`.

## Production readiness

Not declared. This is a lab/dev incremental integration batch. Owner acceptance
and genuinely independent QA are not claimed.
