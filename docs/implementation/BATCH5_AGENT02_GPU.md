# Batch 5 — Agent 02: GPU Telemetry, Admission Control & Gaming

**Date:** 2026-11-25
**Repository:** `D:\Aegis`
**Baseline:** `2bd2513` (uncommitted workspace work preserved)
**Scope:** additive `aegis-fleet` changes only. No commits, no deployment, no
game launch, no model load, no GPU policy default changes.

**Status:** Implemented and tested. Manual idle/inference/gaming capture is
**pending owner participation**. No FPS telemetry exists by default.

---

## 1. Audit before changing

Existing GPU/admission code was read first:

| Area | Location | Behaviour |
|---|---|---|
| Telemetry probe | `crates/aegis-fleet/src/gpu.rs` | `nvidia-smi` CSV row → `GpuTelemetry`; missing data is `available = false`, never fabricated. |
| Admission | `crates/aegis-fleet/src/gpu.rs` | Fail-closed `evaluate_admission_with_gaming`; defaults remain 25% utilization, 2048 MB free VRAM, 80 °C. |
| Yield supervisor | `crates/aegis-fleet/src/supervisor.rs` | Hysteresis, cooperative unload, deferred work, before/after VRAM recovery. |
| Gaming detector | `crates/aegis-fleet/src/gaming.rs` | Process-list signal; a low-utilization menu is still a detected game. |
| Peer modules | `src/freshness.rs`, `src/capabilities.rs` | Already added by another agent; left untouched. |

No admission-policy defaults, global GPU policy, or safety behaviour was
changed. The new module is a **read-only post-processing layer**.

---

## 2. What changed

### 2.1 `crates/aegis-fleet/src/performance.rs` (new)

Pure, serializable, std + existing deps only.

- `TelemetrySeries` — ordered `Vec<GpuTelemetry>` with `summary()` and
  `classification()`.
- `PerformanceSummary` — `schema`, `sample_count`, `available_samples`,
  min/max/avg GPU utilization, min/max/avg used VRAM, min/max/avg free VRAM,
  min/max temperature, observed total VRAM, and total reclaimed VRAM.
- `summarize_telemetry(&[GpuTelemetry]) -> PerformanceSummary` — pure.
- `classify_workload(&PerformanceSummary) -> WorkloadClassification` — heuristic
  labels `idle`, `inference_like`, `gaming_like`, `unknown`, with rationale.
- `render_summary_csv` / `render_summary_report` — deterministic CSV and text.
- `FrameTimeReport`, `parse_frame_times_csv`, `summarize_frame_times`,
  `load_frame_time_report` — manual frame-time ingestion.
- Explicit unavailable path: no file → `available = false`, all metrics `None`.

Reclaimed VRAM is the sum of positive free-VRAM deltas between consecutive
available samples. It approximates recovery across yield boundaries; it cannot
tell an AEGIS unload from an unrelated process freeing memory.

### 2.2 `crates/aegis-fleet/src/lib.rs` (modified)

Added `pub mod performance;` and re-exports. No existing export changed.

### 2.3 `crates/aegis-fleet/tests/performance.rs` (new)

Integration coverage for public exports, the nvidia-smi row → summary path, the
optional CSV path, and an env-gated live snapshot.

---

## 3. Summary CSV format

Deterministic one-header/one-row output, fixed field order:

```text
schema,sample_count,available_samples,
utilization_min_percent,utilization_max_percent,utilization_avg_percent,
used_vram_min_mb,used_vram_max_mb,used_vram_avg_mb,
free_vram_min_mb,free_vram_max_mb,free_vram_avg_mb,
temperature_min_c,temperature_max_c,vram_total_mb,
total_reclaimed_vram_mb,reclaimed_steps
```

Missing values render as empty fields; averages use two decimal places. No
locale-dependent formatting, no external CSV library.

---

## 4. Workload classification heuristics (not ground truth)

Evaluated in this order over **available** samples:

| Label | Rule |
|---|---|
| `idle` | avg utilization < 10% **and** peak utilization < 25% |
| `gaming_like` | avg utilization ≥ 50% **or** peak utilization ≥ 85% **or** (avg VRAM used ≥ 80% of total **and** avg utilization ≥ 30%) |
| `inference_like` | any other set with utilization samples |
| `unknown` | no available utilization samples |

**Important:** telemetry alone cannot distinguish a game from other heavy GPU
work, and a game in a menu/loading/paused state can look idle. These labels are
for the owner’s measurement notes only. They are **not** an admission decision
and must not be used to weaken the fail-closed policy. Gaming detection for
admission remains the explicit process probe in `src/gaming.rs`.

---

## 5. Manual frame-time / FPS ingestion

AEGIS does **not** measure FPS and does not bundle PresentMon or an overlay.
There is no FPS telemetry available by default. The optional input is an
owner-exported CSV with exactly two columns:

```csv
timestamp_ms,frame_time_ms
0,16.02
16,16.71
32,15.98
```

- One leading header row is tolerated; blank lines are ignored.
- Every data row must be two positive, finite numbers.
- `avg_fps = 1000 * frame_count / sum(frame_time_ms)`.
- 1% low = mean of the slowest 1% frame times, converted to FPS
  (`1000 / mean_ms`); 0.1% low uses the slowest 0.1%.
- Max frame time = largest frame time in the capture.
- Missing file → `FrameTimeReport::unavailable(...)`, not fabricated numbers.

`load_frame_time_report(None)` returns an explicit unavailable report. A
malformed or empty file returns unavailable with the parse error.

---

## 6. Owner runbook (manual capture)

All captures are read-only. Do not launch a game or load a model unless you
intend to and can supervise it.

### 6.1 Idle sample set

1. Close games, model servers, and other GPU consumers you can identify.
2. Capture ~30 one-second samples:

```powershell
1..30 | ForEach-Object {
  nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu `
    --format=csv,noheader,nounits
  Start-Sleep -Seconds 1
} | Tee-Object -FilePath .\gpu-idle.csv
```

3. Feed each row through `aegis_fleet::parse_nvidia_smi_line`, collect the
   samples into `TelemetrySeries`, then record `summary()` +
   `classification()` + `render_summary_csv()`.
4. Note the date/time, GPU name, driver version, desktop state, and any
   background apps. If anything else was using the GPU, say so.

### 6.2 Inference-like sample set

1. Start the intended local model only for the measurement window, with owner
   supervision. Respect the existing admission policy.
2. Repeat the same 30-sample capture during actual inference work.
3. Record model name, quantization, prompt/workload shape, concurrent requests,
   and whether the supervisor yielded.

### 6.3 Gaming-like sample set

1. Launch the game manually and play only if you choose to. This agent did not
   and will not launch it.
2. Capture the same 30 samples during real gameplay, not just a menu.
3. If frame data is wanted, export it separately (see 6.4) and keep the GPU
   summary and frame capture time-aligned by recording start/stop clock times.

### 6.4 Frame data export

- PresentMon (GUI or CLI) can export a CSV containing frame timing.
- Export/transform the capture to the two-column format in section 5
  (`timestamp_ms,frame_time_ms`). Column names and units vary by PresentMon
  version; the owner must map them explicitly. This crate intentionally does
  not guess columns.
- Pass the file to `load_frame_time_report(Some(path))`. If no file is provided,
  report FPS as unavailable.

### 6.5 What to record with every capture

- Timestamp and duration; GPU name and driver; display resolution/refresh.
- Whether a game or model was actually running (not just launched).
- Background processes and any other GPU users.
- The exact summary CSV; keep raw samples if possible.
- For frame data: PresentMon version, captured process, and how columns were
  mapped.

---

## 7. Observed data (snapshot, not a benchmark)

Read-only `nvidia-smi` snapshots on 2026-11-25, no model loaded and no game
launched:

| Sample | GPU | Util | Used / Total VRAM | Temp |
|---|---|---|---|---|
| 1 | NVIDIA GeForce RTX 3080 Ti | 4% | 1970 / 12288 MB | 60 °C |
| 2 | NVIDIA GeForce RTX 3080 Ti | 4% | 1970 / 12288 MB | 60 °C |
| 3 | NVIDIA GeForce RTX 3080 Ti | 6% | 1970 / 12288 MB | 60 °C |

The harness rendered: 1 sample, util min/max/avg 4/4/4.00%, used VRAM
1970 MB, free VRAM 10318 MB, temp 60 °C, classification `idle`.

This is a **desktop snapshot**, not a benchmark, not an idle baseline with all
background apps closed, and not evidence about game performance.

---

## 8. Critical limitations

- **No FPS telemetry by default.** `nvidia-smi` does not expose frame times.
  Without an owner-provided capture, FPS is reported unavailable.
- Classification is heuristic and cannot prove gaming-safe operation. A menu or
  loading screen can look `idle`; heavy compute can look `gaming_like`.
- `nvidia-smi` utilization is a coarse sampled metric, not per-frame truth.
- Reclaimed VRAM is an inference from free-VRAM deltas; it is not proof that a
  specific model was unloaded.
- **The previous Phasmophobia test was main-menu only. It does not establish
  gaming-safe operation.** Real gameplay telemetry and frame data are still
  required before any such claim.
- No policy defaults, global GPU policy, or safety thresholds were changed.

---

## 9. Verification

Commands run in `D:\Aegis` (crate-scoped only):

```text
cargo test -p aegis-fleet
  lib:                   41 passed, 0 failed
  integration fleet.rs:  12 passed, 0 failed
  integration performance.rs: 3 passed, 0 failed
  total:                 56 passed, 0 failed
cargo clippy -p aegis-fleet --all-targets -- -D warnings
  clean (no warnings)
```

New tests added: 10 unit tests in `performance.rs` + 3 integration tests in
`tests/performance.rs` (including the env-gated live snapshot).

---

## 10. Files changed

- `crates/aegis-fleet/src/performance.rs` (new)
- `crates/aegis-fleet/src/lib.rs` (module + re-exports only)
- `crates/aegis-fleet/tests/performance.rs` (new)
- `docs/implementation/BATCH5_AGENT02_GPU.md` (this file)

No other crate, workspace file, lockfile, `.gitignore`, CLI, or completion doc
was modified.

---

## 11. Owner actions needed

1. Run the idle and inference captures in section 6 and keep the summary CSVs.
2. Manually run the gaming capture (real gameplay, not a menu) if desired.
3. Export PresentMon/other frame data to `timestamp_ms,frame_time_ms`, or state
   that FPS is unavailable.
4. Decide whether the measured data warrants any policy change. Any change is an
   explicit owner action; this agent did not touch defaults.
5. Treat all current labels as measurement notes, not a gaming-safety claim.
