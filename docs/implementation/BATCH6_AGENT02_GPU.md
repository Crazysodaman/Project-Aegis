# Batch 6 — Agent 02: GPU Measurement Harness & Real-Gameplay Runbook

**Date:** 2026-11-25
**Repository:** `D:\Aegis`
**HEAD:** `2bd2513` (uncommitted workspace work preserved)
**Scope:** additive `crates/aegis-fleet/src/**`, `crates/aegis-fleet/tests/**`, and this document only.

**Status:** Implementation and unit/integration verification complete.
**Real Phasmophobia gameplay data: NONE in Batch 6.** Gameplay capture is
**BLOCKED** until the owner manually starts and plays the game. This agent did
not launch any game and does not claim gameplay verification.

No commit, deploy, reset, rebase, clean, model load/unload, GPU policy change, or
game launch was performed. `D:\SofiaDev\SofiaAdaLyra` was not touched.

---

## 1. Audit before changing

Read first, left intact:

| Area | Location | Behaviour preserved |
|---|---|---|
| Telemetry probe | `src/gpu.rs` | `nvidia-smi` CSV → `GpuTelemetry`; missing data is `available = false`, never invented. |
| Admission | `src/gpu.rs` | Fail-closed `evaluate_admission_with_gaming`; defaults 25% / 2048 MB / 80 °C unchanged. |
| Supervisor | `src/supervisor.rs` | Hysteresis, cooperative unload, deferred work, recovery. |
| Gaming detector | `src/gaming.rs` | Process-list signal. |
| Batch 5 summariser | `src/performance.rs` | Telemetry series, heuristic classification, CSV/report renderers. |
| Agent 01 peers | `src/transport.rs`, `src/deployment.rs`, `src/capabilities.rs`, `src/freshness.rs` | Unchanged. |

New behaviour is additive. Existing public functions keep their signatures
except that frame-time parse/summarise errors are now a **typed error** instead
of `String`; callers that only used `?`/`is_err()`/`expect()` are unaffected.

---

## 2. What changed

### 2.1 `src/measurement.rs` (new)

Pure, serializable, existing dependencies only.

- `ConditionLabel` — `idle`, `inference`, `gaming`, with `parse`/`as_str`.
- `LabeledMeasurement` — a `TelemetrySeries` plus an optional `FrameTimeReport`,
  labelled by condition.
- `compare_conditions(baseline, &[LabeledMeasurement]) -> ComparisonReport` —
  deterministic multi-condition comparison. The baseline series is included with
  zero deltas when present. If no series carries the baseline label, every delta
  is marked `baseline_missing`.
- `ComparisonReport` / `ConditionComparison` — per-series `PerformanceSummary`,
  optional frame report, and a fixed-order `Vec<MetricDelta>`.
- `MetricKind` — VRAM used avg, VRAM free avg, utilization avg/max, temperature
  max, and (when supplied) avg FPS, 1% low, 0.1% low, max frame time, p95, p99.
- `DeltaStatus` — `available`, `baseline_missing`, `baseline_unavailable`,
  `condition_unavailable`. Missing inputs are never silently defaulted.
- `render_comparison_report` — deterministic text; deltas render as
  `condition - baseline` with two-decimal formatting.
- `CaptureConfig`, `CaptureError`, `capture_telemetry_series` — live-capture
  orchestration (see §4).

Delta semantics are fixed and documented:

```text
absolute_delta = condition - baseline
percent_delta  = absolute_delta / baseline * 100   (unavailable when baseline == 0)
```

The report is measurement notes, **not ground truth**, and cannot prove
gaming-safe operation.

### 2.2 `src/performance.rs` (upgraded, additive)

- `FrameTimeReport` gains `p95_frame_time_ms` and `p99_frame_time_ms`.
- Percentiles use the documented nearest-rank method over ascending frame times
  (`ceil(p * count)`, clamped).
- `FrameTimeError` (typed, `thiserror`) with variants `Empty`, `FieldCount`,
  `Timestamp`, `FrameTime`, `NonFiniteFrameTime`, `NonPositiveFrameTime`,
  `InvalidSample`. Malformed rows are rejected at the exact line; a single
  leading non-numeric header row is still tolerated, blank lines ignored.
- `summarize_frame_times` / `parse_frame_times_csv` now return
  `Result<_, FrameTimeError>`.
- Existing metrics retained: avg FPS, 1% low, 0.1% low, max frame time. No value
  is fabricated for missing captures.

### 2.3 `src/bin/aegis-fleet-measure.rs` (new)

Read-only harness binary:

- `capture` samples `nvidia-smi` via `NvidiaSmiProbe` and prints a summary. It
  loads no model, launches no game, and changes no policy.
- `compare` ingests owner-captured snapshot CSVs plus optional frame CSVs and
  prints the comparison report (text or `--json`).
- This is a thin wrapper over the pure library; the Lead can add equivalent
  `aegis-cli` subcommands later without duplicating logic.

### 2.4 `src/lib.rs` (module wiring + re-exports only)

Added `pub mod measurement;` and re-exported the new types/functions plus
`FrameTimeError`. No existing export was removed or renamed.

### 2.5 `tests/measurement.rs` (new)

Integration coverage for the public comparison API, frame fixture percentiles,
and the synthetic-probe capture helper.

---

## 3. Comparison API example

```rust
use aegis_fleet::{
    compare_conditions, render_comparison_report, summarize_frame_times,
    ConditionLabel, LabeledMeasurement, TelemetrySeries,
};

let idle = LabeledMeasurement::new(ConditionLabel::Idle, idle_series);
let inference = LabeledMeasurement::new(ConditionLabel::Inference, inference_series)
    .with_frame_times(summarize_frame_times(&frame_ms, "presentmon").expect("frames"));
let gaming = LabeledMeasurement::new(ConditionLabel::Gaming, gaming_series);

let report = compare_conditions(ConditionLabel::Idle, &[idle, inference, gaming]);
print!("{}", render_comparison_report(&report));
println!("{}", serde_json::to_string_pretty(&report)?);
```

Determinism: fixed metric order, no hash maps, no locale-dependent formatting,
and identical input produces identical output (covered by a test).

---

## 4. Live-capture helper design

```rust
pub fn capture_telemetry_series<P, F>(
    probe: &P,
    config: CaptureConfig,   // sample_count + interval
    wait: F,                 // FnMut(Duration); the only delay source
) -> Result<TelemetrySeries, CaptureError>
where
    P: GpuProbe + ?Sized,
    F: FnMut(Duration),
```

- Production callers pass `std::thread::sleep`; tests pass a recorder, so unit
  tests capture a synthetic probe with **no real delay**.
- Config bounds: `1..=3600` samples, interval `<= 3_600_000 ms`.
- Unavailable probe samples are stored as unavailable; nothing is filtered or
  fabricated.
- No model load, no game launch, no GPU policy change.

The binary uses this exact path with `NvidiaSmiProbe`.

---

## 5. Harness invocations (exact)

Run from `D:\Aegis`. These are read-only. `capture` only calls `nvidia-smi`.

### 5.1 Capture a snapshot set

```powershell
cargo run -p aegis-fleet --bin aegis-fleet-measure -- `
  capture --samples 30 --interval-ms 1000
```

JSON form (for archiving raw series):

```powershell
cargo run -p aegis-fleet --bin aegis-fleet-measure -- `
  capture --samples 30 --interval-ms 1000 --json | Out-File .\gpu-capture.json -Encoding utf8
```

### 5.2 Compare conditions from captured CSVs

Each snapshot file is the raw `nvidia-smi --format=csv,noheader,nounits` output,
one row per sample, no header:

```text
NVIDIA GeForce RTX 3080 Ti, 3, 1945, 12288, 59
```

```powershell
cargo run -p aegis-fleet --bin aegis-fleet-measure -- `
  compare --baseline idle `
    --idle .\gpu-idle.csv `
    --inference .\gpu-inference.csv `
    --gaming .\gpu-gaming.csv `
    --inference-frames .\frames-inference.csv `
    --gaming-frames .\frames-gaming.csv
```

Add `--json` for a serializable report. Missing files/labels are simply omitted
and their deltas render as unavailable; an omitted baseline marks every delta
`baseline_missing`.

---

## 6. Owner runbook — real gameplay capture (manual, blocked until owner)

**This agent did not and will not launch Phasmophobia.** Follow only if you
choose to participate. All steps are read-only measurements.

### 6.1 Preparation

1. Confirm no model server is running unless the inference condition requires it.
2. Identify the game process/executable and the display refresh rate.
3. Decide capture window: ~30 samples at 1 s (or longer for gameplay).
4. Keep a notes file with date/time, GPU driver, desktop state, and every running
   GPU consumer.

### 6.2 Idle capture

1. Close games and idle the GPU.
2. Run:

```powershell
cargo run -p aegis-fleet --bin aegis-fleet-measure -- `
  capture --samples 30 --interval-ms 1000 --json | Out-File .\gpu-idle.json -Encoding utf8
```

3. Archive the raw series and the rendered summary.

### 6.3 Inference capture (optional, owner-supervised)

1. Start the intended local model **yourself** for the measurement window only,
   respecting the existing admission policy.
2. Capture 30 samples the same way, while real inference work is running.
3. Record model name, quantization, prompt shape, concurrency, and whether the
   supervisor yielded.

### 6.4 Gameplay capture

1. **Owner manually starts the game.** The agent never does this.
2. Play real gameplay, not a menu/loading screen. Note start/stop clock times.
3. Start the capture immediately before or during gameplay:

```powershell
cargo run -p aegis-fleet --bin aegis-fleet-measure -- `
  capture --samples 120 --interval-ms 500 --json | Out-File .\gpu-gaming.json -Encoding utf8
```

4. Stop the capture, then stop the game yourself.

### 6.5 PresentMon frame export

1. Capture frames with PresentMon for the same window/process.
2. Export CSV. Column names vary by PresentMon version.
3. Map to exactly two columns and units:

```csv
timestamp_ms,frame_time_ms
0,16.02
16,16.71
```

   - `frame_time_ms` is the per-frame duration (PresentMon's
     `msBetweenPresents` class of column, in milliseconds).
   - `timestamp_ms` is a non-negative milliseconds timestamp (convert seconds
     with `* 1000`).
   - Do not reorder, interpolate, or hand-edit values.

4. Feed the file with `--gaming-frames` (or `--idle-frames` / `--inference-frames`).
   If no frame file exists, FPS stays **unavailable**; nothing is fabricated.

### 6.6 Compare

```powershell
cargo run -p aegis-fleet --bin aegis-fleet-measure -- `
  compare --baseline idle `
    --idle .\gpu-idle.csv `
    --inference .\gpu-inference.csv `
    --gaming .\gpu-gaming.csv `
    --inference-frames .\frames-inference.csv `
    --gaming-frames .\frames-gaming.csv `
  | Tee-Object -FilePath .\comparison-report.txt
```

### 6.7 What to record with every capture

- Date/time, duration, start/stop clocks.
- GPU name, driver version, display resolution/refresh.
- Whether a game or model was **actually running**, not merely launched.
- All background GPU consumers.
- Raw series (`--json`), rendered summary, and the comparison report.
- PresentMon version, captured process, and exact column mapping.
- Any suspect data (menu-only, alt-tab, loading stall) flagged explicitly.

---

## 7. Observed snapshot (evidence only, **not a benchmark**)

Read-only `nvidia-smi` on 2026-11-25. No model loaded, no game launched.

| Source | GPU | Util | Used / Total VRAM | Temp |
|---|---|---|---|---|
| Direct `nvidia-smi` | NVIDIA GeForce RTX 3080 Ti | 6% | 1931 / 12288 MB | 60 °C |
| Harness `capture` | NVIDIA GeForce RTX 3080 Ti | 3% | 1945 / 12288 MB | 59 °C |

Harness summary: 1 sample, util 3%, free VRAM 10343 MB, classification `idle`.

This is a **desktop snapshot**, not a benchmark, not an idle baseline with all
background apps closed, and not evidence about game performance.

---

## 8. Verification

Crate-scoped only; no whole-workspace gates.

```text
cargo test -p aegis-fleet
  src/lib.rs unit tests:        75 passed, 0 failed
  src/bin aegis-fleet-identity:  0 tests
  src/bin aegis-fleet-measure:   0 tests
  tests/fleet.rs:               13 passed, 0 failed
  tests/measurement.rs:          3 passed, 0 failed
  tests/performance.rs:          3 passed, 0 failed
  doc-tests:                     0 tests
  total executable tests:       94 passed, 0 failed

cargo clippy -p aegis-fleet --all-targets -- -D warnings
  clean (exit 0, no warnings)
```

New tests:

- `measurement.rs` unit: 9 (comparison deltas, missing baseline, missing frame
  data, determinism + serde round-trip, empty input, label round-trip, capture
  via synthetic probe, invalid capture config, unavailable-sample capture).
- `performance.rs` unit: 2 new (`FrameTimeError` variants/lines, nearest-rank
  p95/p99) plus updated existing frame tests.
- `tests/measurement.rs` integration: 3.

No existing test was weakened; fleet/transport/deployment/supervisor tests still
pass.

---

## 9. Files changed

- `crates/aegis-fleet/src/measurement.rs` (new)
- `crates/aegis-fleet/src/bin/aegis-fleet-measure.rs` (new)
- `crates/aegis-fleet/src/performance.rs` (additive frame-time upgrade)
- `crates/aegis-fleet/src/lib.rs` (module wiring + re-exports only)
- `crates/aegis-fleet/tests/measurement.rs` (new)
- `docs/implementation/BATCH6_AGENT02_GPU.md` (this file)

Not touched: `crates/aegis-cli`, workspace `Cargo.toml`, `Cargo.lock`,
`.gitignore`, `docs/completion`, any other crate, `D:\SofiaDev\SofiaAdaLyra`.
No new dependencies.

---

## 10. Blocked items and limitations

- **Real gameplay verification is BLOCKED.** Batch 6 contains **no** Phasmophobia
  gameplay telemetry or frame data. Nothing here claims otherwise. Only manual
  owner participation can unblock it.
- No GPU policy defaults, thresholds, or safety behaviour were changed.
- Classification remains heuristic: a menu can look `idle`; heavy compute can
  look `gaming_like`; `nvidia-smi` utilization is coarse and sampled.
- Percentiles/lows are only as good as the owner-supplied frame CSV; PresentMon
  column mapping is the owner's responsibility.
- Reclaimed VRAM (Batch 5) is an inference from free-VRAM deltas, not proof of a
  specific unload.
- The harness binary is a lab measurement tool, not a benchmark or safety gate.

---

## 11. Owner actions to unblock real gameplay evidence

1. Run the idle capture in §6.2 and archive the JSON.
2. Optionally run the supervised inference capture in §6.3.
3. Manually start and play Phasmophobia, then capture per §6.4.
4. Export PresentMon frames to `timestamp_ms,frame_time_ms` per §6.5.
5. Run the comparison in §6.6 and store the report.
6. Decide explicitly whether any measured data justifies a policy change; this
   agent made none.

Until steps 3–5 are done by the owner, the correct status is **no gameplay data /
not verified**.
