# Batch 7 — Agent 02: GPU Measurement Harness & Real Inference-Pressure Capture

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**HEAD:** `2bd2513` (all uncommitted workspace work preserved)
**Scope:** additive edits under `crates/aegis-fleet/src/**`, `crates/aegis-fleet/tests/**`, plus this document only.
**Machine:** Venus — NVIDIA GeForce RTX 3080 Ti, driver 616.64, 12288 MiB, Windows 11 Home.
**Runtime used:** Ollama 0.40.17 with the already-installed `qwen3.5:9b` (no download).

**Status:** Implementation + unit/integration verification complete. A **real
inference-pressure measurement was captured on Venus** (idle → during generation
→ after generation → after unload). This is **inference pressure, NOT gameplay**.
No game was launched or played. **Real FPS / frame-time evidence remains
BLOCKED** on manual owner participation and is not claimed here.

No commit, deploy, reset, rebase, clean, destructive git, GPU-policy change, or
model download occurred. `D:\SofiaDev\SofiaAdaLyra` was not touched.

---

## 1. Audit before changing

Read first and left behaviour intact:

| Area | Location | Preserved |
|---|---|---|
| Telemetry probe | `src/gpu.rs` | `nvidia-smi` CSV → `GpuTelemetry`; missing data stays `available = false`. |
| Admission | `src/gpu.rs` | Fail-closed `evaluate_admission_with_gaming`; defaults 25 % / 2048 MB / 80 °C unchanged. |
| Supervisor | `src/supervisor.rs` | Hysteresis, cooperative unload, deferred work, recovery. |
| Summariser | `src/performance.rs` | Series summary, heuristic classification, CSV/report renderers. |
| Comparison | `src/measurement.rs` | `compare_conditions`, `CaptureConfig`, `capture_telemetry_series`. |
| Harness bin | `src/bin/aegis-fleet-measure.rs` | `capture` and `compare` subcommands unchanged. |

All changes are additive; no existing public signature or default was changed.

---

## 2. What changed (harness improvement)

Comparing idle vs inference pressure reproducibly needs the four-phase episode
(idle, during generation, after generation with the model resident, after
unload) to be reduced by one deterministic, testable rule set. That logic was
missing, so it was added as pure library code plus a thin CLI subcommand.

### 2.1 `src/measurement.rs` — new pure logic

- `PRESSURE_REPORT_SCHEMA = "aegis.fleet.inference_pressure_report.v1"`.
- `PressurePhase` — per-phase `sample_count`, `available_samples`, used-VRAM
  min/max/avg, free-VRAM avg, utilization avg/max, temperature max.
- `PressureReport` — the four labelled phases plus deterministic deltas:
  - `inference_vram_growth_mb = during.used_avg - idle.used_avg`
  - `retained_vram_mb        = after.used_avg  - idle.used_avg`
  - `recovered_vram_mb       = after.used_avg  - unloaded.used_avg`
  - `recovered_fraction      = recovered / retained` (only when retained > 0)
  - `utilization_growth_percent = during.util_avg - idle.util_avg`
- `summarize_pressure(idle, during, after, unloaded) -> PressureReport`.
  Pure and deterministic; never probes, loads a model, starts a game, or
  fabricates a value. Every delta is `Option` and stays `None` when either
  operand is unavailable.
- `render_pressure_report(&PressureReport) -> String` — explicit
  `unavailable` markers, no locale-dependent formatting.

### 2.2 `src/bin/aegis-fleet-measure.rs` — new `pressure` subcommand

```text
aegis-fleet-measure pressure --idle <csv> --during <csv> --after <csv> --unloaded <csv> [--json]
```

Each CSV is raw `nvidia-smi --format=csv,noheader,nounits` output (one row per
sample, no header), the same format `compare` already consumes. The binary is
still a thin read-only wrapper: it reads files and prints a report. It does not
invoke Ollama or start inference. `capture` and `compare` usage text unchanged.

### 2.3 `src/lib.rs`

Re-exports only: `summarize_pressure`, `render_pressure_report`,
`PressurePhase`, `PressureReport`, `PRESSURE_REPORT_SCHEMA`. Nothing removed or
renamed.

### 2.4 Tests

- `src/measurement.rs` unit tests (3 new): growth/retention/recovery arithmetic
  and renderer, missing-phase data explicitly unavailable, determinism +
  serde round-trip.
- `tests/measurement.rs` integration test (1 new): pressure episode through the
  public API.

---

## 3. Real inference-pressure measurement on Venus

### 3.1 Why this is NOT gameplay

Only `nvidia-smi` snapshots and one local text generation were used. No game
process was started, no FPS counter was run, no frame times were captured. GPU
utilization of an LLM generation is **not** a game workload and says nothing
about frame rate, frame pacing, or whether a game is safe to run. The
`classify_workload` heuristic would label this interval `gaming_like` (avg
utilization 72 %), which is exactly why that heuristic must not be used as a
safety gate or as gameplay evidence.

### 3.2 Capture procedure (exact steps)

Model already installed: `qwen3.5:9b` (5.9 GB, shown by `ollama ps` during the
run). Nothing was downloaded. The capture loop writes raw `nvidia-smi` rows:

```powershell
$smi = (Get-Command nvidia-smi).Source
$q = @('--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu',
       '--format=csv,noheader,nounits')
function Capture-Rows($path,$count) {
  if (Test-Path $path) { Remove-Item -LiteralPath $path }
  for ($i=0; $i -lt $count; $i++) {
    $line = & $smi @q | Select-Object -First 1
    Add-Content -LiteralPath $path -Value $line -Encoding ascii
    if ($i -lt $count-1) { Start-Sleep -Milliseconds 1000 }
  }
}
```

Exact sequence used:

1. Clean start / idle baseline.
   ```powershell
   ollama stop qwen3.5:9b          # no-op, nothing loaded
   Capture-Rows .\idle.csv 20      # 1 s interval
   ```
2. Start generation in a background job, wait for `ollama ps` to show the model,
   then sample once per second **while the job is still Running**:
   ```powershell
   $ollama = (Get-Command ollama).Source
   $prompt = 'Write a very long, detailed 2000-word technical explanation of how transformer self-attention works, ... do not stop early.'
   $job = Start-Job { param($e,$p) & $e run qwen3.5:9b $p } -ArgumentList $ollama,$prompt
   # poll `ollama ps` until it lists qwen3.5:9b
   # then, while ($job.State -eq 'Running') { sample nvidia-smi; sleep 1s }  -> during.csv
   Wait-Job $job; Receive-Job $job; Remove-Job $job
   ```
3. After generation with the model still resident:
   ```powershell
   Capture-Rows .\after.csv 15
   ```
4. Unload and capture recovery:
   ```powershell
   ollama stop qwen3.5:9b
   Start-Sleep -Seconds 3
   Capture-Rows .\unloaded.csv 15
   ollama ps                      # must be empty
   ```
5. Reduce with the new harness command:
   ```powershell
   cargo run -q -p aegis-fleet --bin aegis-fleet-measure -- `
     pressure --idle .\idle.csv --during .\during.csv `
              --after .\after.csv --unloaded .\unloaded.csv
   ```

`ollama ps` during generation:

```text
NAME          ID              SIZE      PROCESSOR    CONTEXT    RUNNER      UNTIL
qwen3.5:9b    c97eb11d70b1    5.9 GB    100% GPU     16384      llamacpp    4 minutes from now
```

### 3.3 Measured values (real, Venus, 2026-11-26)

| Phase | Samples | Util avg | Util max | Used VRAM avg | Free VRAM avg | Temp max |
|---|---|---|---|---|---|---|
| idle | 20 | 1.15 % | 11 % | 1941 MB | 10347 MB | 59 °C |
| inference during generation | 48 | 72.38 % | 80 % | 8742 MB | 3546 MB | 78 °C |
| after generation (model resident) | 15 | 2.47 % | 5 % | 8742 MB | 3546 MB | 65 °C |
| after unload | 15 | 0.87 % | 4 % | 1941 MB | 10347 MB | 57 °C |

Derived ledger from the harness:

```text
inference VRAM growth:            6801.00 MB   (during - idle)
retained VRAM while resident:     6801.00 MB   (after  - idle)
recovered VRAM after unload:      6801.00 MB   (after  - unloaded)
recovered fraction of retained:   1.00
utilization growth:               71.22 percentage points (during - idle)
```

Observations:

- Used VRAM was flat at 8742 MB for the entire generation and stayed at
  8742 MB after generation completed while the model remained resident, i.e.
  the weights + KV cache were retained, not released between tokens.
- `ollama stop qwen3.5:9b` returned used VRAM exactly to the idle baseline
  (8742 → 1941 MB), i.e. all 6801 MB of inference pressure was recovered.
- Temperature rose from 59 °C idle to 78 °C peak under generation and decayed
  to 51–57 °C after unload.
- During-generation utilization is spiky (54–80 %), consistent with
  token-by-token GPU work; this is not frame delivery.

---

## 4. Cleanup confirmation

- `ollama ps` before the run: empty; during: `qwen3.5:9b` 100 % GPU; after
  `ollama stop`: **empty**; final `ollama ps`: **empty**.
- `ps-after-stop.txt` and the final `ollama ps` both show only the header row —
  **no model left loaded**.
- No `ollama run` child process remained (the background job was waited on,
  drained, and removed). The pre-existing `ollama` server / `ollama app`
  processes and unrelated pre-existing `aegis-core-host` processes were not
  touched.
- Post-run `nvidia-smi`: utilization 2 %, used 1941 / 12288 MB, 51 °C — back at
  the idle baseline.
- Capture files were written only under
  `C:\Users\kenny\AppData\Local\Temp\opencode\batch7gpu\`; nothing was added to
  the repository for the measurement.

---

## 5. Verification

Crate-scoped only; no whole-workspace gates.

```text
cargo test -p aegis-fleet
  src/lib.rs unit tests:                 93 passed, 0 failed
  src/bin aegis-fleet-identity:           0 tests
  src/bin aegis-fleet-measure:            0 tests
  tests/fleet.rs:                        13 passed, 0 failed
  tests/live_identity.rs:                 1 passed, 0 failed
  tests/measurement.rs:                   4 passed, 0 failed
  tests/performance.rs:                   3 passed, 0 failed
  doc-tests:                              0 tests
  total executable tests:               114 passed, 0 failed

cargo clippy -p aegis-fleet --all-targets -- -D warnings
  clean (exit 0, no warnings)
```

New tests:

- `measurement.rs` unit: `pressure_computes_growth_retained_and_recovered`,
  `pressure_missing_phase_data_is_explicitly_unavailable`,
  `pressure_is_deterministic_and_serializable`.
- `tests/measurement.rs`:
  `pressure_episode_reports_growth_retention_and_recovery_via_public_api`.

No existing test was weakened; fleet/identity/performance tests still pass.

---

## 6. Files changed

- `crates/aegis-fleet/src/measurement.rs` — pressure types + pure summariser +
  renderer + 3 unit tests.
- `crates/aegis-fleet/src/bin/aegis-fleet-measure.rs` — `pressure` subcommand.
- `crates/aegis-fleet/src/lib.rs` — re-exports only.
- `crates/aegis-fleet/tests/measurement.rs` — 1 integration test.
- `docs/implementation/BATCH7_AGENT02_GPU.md` — this file.

Not touched: `crates/aegis-cli`, workspace `Cargo.toml`, `Cargo.lock`,
`.gitignore`, `docs/completion`, any other crate, `D:\SofiaDev\SofiaAdaLyra`.
No new dependencies (the harness reads files only; no HTTP client was added).

---

## 7. Blocked items and limitations

- **Real gameplay / FPS / frame-time verification is BLOCKED.** Batch 7 contains
  **no** game telemetry or frame data. Only inference pressure was measured.
  Nothing here claims a game is safe or that any frame rate is acceptable.
- The workload classifier is heuristic and cannot distinguish LLM inference from
  gaming; do not use it as a safety gate.
- `nvidia-smi` utilization is coarse and sampled at 1 Hz; spikes between samples
  are not visible.
- The retained/recovered figures are VRAM deltas around a cooperative unload,
  not proof that only the model freed that memory.
- GPU policy defaults, thresholds, and safety behaviour were not changed.
- The pressure ledger is a lab measurement tool, not a benchmark.

---

## 8. Owner runbook — capture real gameplay later (manual, blocked until owner)

This agent did not and will not launch any game.

### 8.1 Preparation

1. Confirm no model is loaded: `ollama ps` should print only its header. If not,
   `ollama stop <model>`.
2. Record date/time, GPU driver, display resolution/refresh rate, and every
   background GPU consumer.
3. Decide the capture window (e.g. 120 samples at 500 ms for gameplay).

### 8.2 Idle baseline

```powershell
cargo run -q -p aegis-fleet --bin aegis-fleet-measure -- `
  capture --samples 30 --interval-ms 1000 --json | Out-File .\gpu-idle.json -Encoding utf8
```

### 8.3 Real gameplay (owner manually starts and plays)

1. **You** start the game and play real gameplay (not a menu/loading screen).
   Note start/stop clock times.
2. Immediately run the harness capture while playing:

```powershell
cargo run -q -p aegis-fleet --bin aegis-fleet-measure -- `
  capture --samples 120 --interval-ms 500 --json | Out-File .\gpu-gaming.json -Encoding utf8
```

3. Stop the capture, then stop the game yourself.

### 8.4 Frame times (FPS)

1. Capture frames with PresentMon for the same window/process.
2. Export to exactly two columns, milliseconds:

```csv
timestamp_ms,frame_time_ms
0,16.02
16,16.71
```

3. Do not reorder, interpolate, or hand-edit values. If no frame CSV exists,
   FPS stays `unavailable`; nothing is fabricated.
4. Derive the raw `nvidia-smi` CSV snapshots for `compare`/`pressure` from the
   same window (one row per sample, `--format=csv,noheader,nounits`).

### 8.5 Compare idle vs inference vs gaming

```powershell
cargo run -q -p aegis-fleet --bin aegis-fleet-measure -- `
  compare --baseline idle `
    --idle .\gpu-idle.csv `
    --inference .\gpu-inference.csv `
    --gaming .\gpu-gaming.csv `
    --inference-frames .\frames-inference.csv `
    --gaming-frames .\frames-gaming.csv
```

For the four-phase inference ledger (no game):

```powershell
cargo run -q -p aegis-fleet --bin aegis-fleet-measure -- `
  pressure --idle .\idle.csv --during .\during.csv `
           --after .\after.csv --unloaded .\unloaded.csv
```

### 8.6 Decide explicitly

Review the measured data yourself. This agent made **no** policy change and
recommends none from inference data alone. Until real gameplay + frame-time
captures exist, the correct status is **no gameplay data / not verified**.
