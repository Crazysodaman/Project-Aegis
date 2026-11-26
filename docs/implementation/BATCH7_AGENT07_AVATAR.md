# Batch 7 - Agent 07: Avatar Frame Sequences, Expressions, and Lip-Sync Hook

Status: **implemented and verified** in `aegis-embodiment` only. This batch is
additive: it renders deterministic CPU BMP frame sequences with the existing
Batch 6 rasterizer, writes them with a bounded manifest, maps a small synthetic
expression/gesture set to overlay parameters, and adds a pure deterministic
lip-sync hook.

No approved Core appearance, Core identity, or Core #7 content was invented or
changed. Every scene in the new tests is synthetic (`00000000-0000-4000-8000-0000000000aa`)
and uses the already-approved default Aegis theme colors; no real person or Core
#7 identity is referenced.

Scope respected: only `crates/aegis-embodiment/src/**`,
`crates/aegis-embodiment/tests/**`, and this document were changed. No
dependencies, downloads, installs, unsafe code, commits, or destructive git
operations. `D:\SofiaDev\SofiaAdaLyra` was not touched.

## 1. Audit (before changes)

Inspected the Batch 6 avatar path first:

- `crates/aegis-embodiment/src/raster.rs` - dependency-free CPU rasterizer that
  composites scene layers as role-colored rectangles and encodes real 24-bit
  uncompressed BMP pixels (bottom-up rows, BGR, 4-byte stride). It already
  supports an optional `AnimationState` transform via `render_bmp_at`.
- `crates/aegis-embodiment/src/animation.rs` - validated `AnimationClip`,
  keyframes/easing, and the pure `AnimationPlayer` (`seek`/`advance`).
- `crates/aegis-embodiment/src/scene.rs` - validated `AvatarScene` with theme,
  layers, and optional expression/gesture motions.
- `crates/aegis-embodiment/src/render.rs` - headless scene description renderer
  (no pixels), kept unchanged.
- `crates/aegis-embodiment/src/lib.rs` - module wiring and re-exports.
- Existing tests: `tests/bmp.rs`, `tests/animation.rs`, `tests/scene.rs`,
  `tests/render.rs` (all still pass).

The missing pieces were a frame-sequence pipeline, a bounded sequence writer with
a manifest, a pure expression/gesture parameter map, and a lip-sync hook. All new
work is additive; `raster.rs`, `scene.rs`, `animation.rs`, `render.rs`, `voice.rs`,
`remote.rs`, and the bins were not modified.

## 2. Exact files changed

Added:

- `crates/aegis-embodiment/src/sequence.rs`
  - `RenderedFrame { index, time_ms, bmp, sha256 }`
  - `render_sequence(scene, clip, width, height, duration_ms, fps)` using
    `AnimationPlayer::seek` + `render_bmp_at`.
  - `sequence_frame_count` and `frame_time_ms` deterministic timing helpers.
  - `write_sequence` / `write_sequence_capped` plus
    `SequenceManifest` / `SequenceFrameEntry` with `frame-NNNNNN.bmp` files and
    `manifest.json`.
  - Bounds: fps 1..=120, duration 1 ms..=1 h, max 7,200 frames, default 256 MiB
    total-size cap.
- `crates/aegis-embodiment/src/expression.rs`
  - `Expression` (neutral/happy/sad/surprised/angry/thinking) and
    `expression_params` pure map to overlay RGB, blend weight, mouth/eye/brow
    parameters.
  - `Gesture` and `gesture_params` pure map to overlay/offset/scale parameters.
  - `render_expression_bmp` renders with the BMP rasterizer and blends the mapped
    overlay into a schematic mouth region.
- `crates/aegis-embodiment/src/lipsync.rs`
  - `plan_lip_sync(transcript, timings, fps, duration_ms)` producing
    `LipSyncPlan { frame_count, cues, mouth_frames }`.
  - Deterministic even distribution or validated caller-supplied `WordTiming`s.
  - `MouthShape` cycle; explicit doc that this is **not audio-synced** and not
    integrated with live audio.
- `crates/aegis-embodiment/tests/sequence.rs` - 9 tests.
- `crates/aegis-embodiment/tests/expression.rs` - 4 tests.
- `crates/aegis-embodiment/tests/lipsync.rs` - 5 tests.
- `crates/aegis-embodiment/tests/avatar_sequence.rs` - 1 live-gated test.
- `docs/implementation/BATCH7_AGENT07_AVATAR.md` - this report.

Edited (additive wiring only):

- `crates/aegis-embodiment/src/lib.rs` - declares `pub mod expression;`,
  `pub mod lipsync;`, `pub mod sequence;`, updates module docs, and re-exports
  the new public API and constants.

No other crate, workspace manifest, `Cargo.lock`, `.gitignore`, CLI, completion
doc, or `D:\SofiaDev\SofiaAdaLyra` content was touched. Existing uncommitted work
in `asset.rs`, `store.rs`, and the prior `lib.rs` changes was preserved.

## 3. Design notes

- Frame timing is integer and deterministic: frame count is
  `ceil(duration_ms * fps / 1000)`; each frame time is `index * 1000 / fps`.
  Identical inputs produce identical BMP bytes and SHA-256 values.
- The writer validates clip timing, frame count, frame index order, and each
  frame digest before writing; it computes the total size and rejects an oversize
  sequence **before creating the output directory**, so a failed cap check leaves
  no partial output.
- Frame files are named `frame-000000.bmp`, `frame-000001.bmp`, ... and the
  manifest records `{schema, fps, duration_ms, width, height, frame_count,
  total_bytes, frames:[{index,file,time_ms,bytes,sha256}]}`.
- The expression overlay patches the encoded BMP pixel buffer directly (BGR,
  bottom-up, stride-aware), blending the mapped color over a fixed schematic
  mouth rectangle. Different expressions produce different deterministic pixels.
- Lip-sync maps words to representative frame indices (word midpoint) and fills a
  per-frame mouth track. Silent gaps are `Closed`. Word timings are validated for
  non-empty, in-bounds, non-overlapping spans.

## 4. Tests and counts

New tests (19 total):

- `tests/sequence.rs` (9): frame count/time determinism, fps/duration bounds,
  movement clip produces differing frames, static clip produces identical frames,
  per-frame SHA-256 matches BMP, writer persists frames + manifest (bytes/time/
  digest/file contents), oversize cap rejected before writing, tampered digest/
  index rejected, mismatched frame count rejected.
- `tests/expression.rs` (4): pure/distinct expression mapping, pure/distinct
  gesture mapping, deterministic difference between rendered expressions with
  valid BMP headers, invalid dimensions propagated.
- `tests/lipsync.rs` (5): deterministic even distribution and bounds, explicit
  timings respected with silent gaps, invalid transcripts/timing bounds rejected,
  invalid per-word timings (mismatch/zero/overlap/out-of-range) rejected, mouth
  frames bounded.
- `tests/avatar_sequence.rs` (1, live-gated): synthetic neutral + happy movement
  sequences, at least 3 frames, differing hashes, valid BMPs, writer manifests.

Package verification:

```powershell
cargo test -p aegis-embodiment
cargo clippy -p aegis-embodiment --all-targets -- -D warnings
```

- `cargo test -p aegis-embodiment`: **94 passed, 0 failed, 1 ignored**
  (the ignored test is the pre-existing LAN test in `tests/artifact_node.rs`;
  doc-tests 0). The live test reports `ok` but skips unless the env gate is set.
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings`: **clean**.

## 5. Live evidence

Command run:

```powershell
$env:AEGIS_AVATAR_LIVE='1'; cargo test -p aegis-embodiment live_avatar_sequence -- --nocapture
```

Result: **1 passed**, 0 failed. Both sequences used a 128x160 synthetic scene,
the approved default Aegis theme, and a 500 ms, 12 fps movement clip.

- Neutral expression: **6 frames**, total **368,964 bytes** (61,494 bytes/frame).
- Happy expression: **6 frames**, total **368,964 bytes** (61,494 bytes/frame).

Sample SHA-256 values:

- Neutral frame 0 (`time_ms=0`):
  `9f2c13b1320dc8b8f985b0ac948854f38ae60e92f0695d0ea1474e189b4da332`
- Neutral frame 1 (`time_ms=83`):
  `c2be98b10b9a9407fa64dfc7665cb981d8664e8415737d7752c888327c7ec62f`
- Happy frame 0 (`time_ms=0`):
  `07e27c3970ea7d957bdf36887db9ae6465b2bebb0f8e131de0b41ab27a3c3267`
- Happy frame 1 (`time_ms=83`):
  `935e85ad67353c5dfc0d275a0df3e3fe5512f2cc260f1eb50235f4175966574b`

The neutral and happy frame sets differ, and within each set the movement clip
produces differing hashes; every frame is a valid BMP whose declared size equals
its byte length. Output was written under a `tempfile` temporary directory that
is removed when the test finishes.

## 6. What is real

- Real CPU-computed RGB pixels encoded as valid 24-bit BMP frames.
- Deterministic frame timing from `AnimationClip` + fps + duration.
- Frame-by-frame SHA-256 integrity and a JSON manifest with index/time/bytes/SHA.
- Enforced total-size cap and per-frame validation before/while writing.
- Deterministic pure expression and gesture parameter maps, with expression
  overlay pixels that visibly change the rendered BMP.
- Deterministic transcript-to-mouth-frame mapping with bounded frame indices and
  optional validated per-word timings.

## 7. What is NOT implemented (honest scope)

- No GPU rendering or acceleration.
- No native window or GUI integration.
- No PNG/JPEG asset decoding; layer rectangles remain schematic.
- No real avatar art, skinning, rigging, or high-quality rasterization.
- No video encoding (frames are individual BMP files, not MP4/GIF/WebM).
- No audio-synced lip-sync, visemes, phoneme alignment, microphone capture, or
  integration with live audio; the lip-sync here is a pure text/timing hook.
- No Core #7 identity/appearance or real personal identity.
- No TUI/GUI embedding of the generated frames.

## 8. Dependencies required for real integration (owner approval needed)

These would be **new dependencies** and were not added by this batch:

- Image decoding/compositing (PNG/JPEG): e.g. `image`, `png`, `resvg`.
- GPU rendering: e.g. `wgpu`.
- Native window/GUI shell: e.g. `winit` + `wgpu`/`egui`/`eframe`.
- Video/animation export: e.g. an encoder crate for MP4/GIF/WebM.
- Audio-synced lip-sync/visemes: an audio analysis or forced-alignment stack
  plus a real audio capture/integration path.

Until approved, the only pixel path is the existing dependency-free BMP
rasterizer and this frame-sequence/manifest writer.

## 9. Limitations

- Expression and gesture parameters are synthetic placeholders, not approved art
  direction; the overlay is a fixed schematic mouth rectangle.
- Movement is limited to translate/scale/opacity on the same rectangles; there is
  no skeletal animation.
- Lip-sync is heuristic: it does not listen to audio and does not claim phoneme
  accuracy.
- The writer is bounded and pre-validated but not transactional; an I/O failure
  mid-write could leave some already-written frame files.
- BMP output has no alpha channel; transparency remains approximated by blending.

## 10. Next steps (pending owner approval)

1. Decide whether image decoding/compositing (`image`/`png`) may be added so real
   asset art can replace schematic rectangles.
2. If a live preview is required, approve a GPU/window stack behind a feature
   flag and keep the CPU BMP path as the headless fallback.
3. Approve a video/sequence container so BMP frames can be muxed into a playable
   clip.
4. Approve an audio-synced lip-sync stack before replacing the pure hook.
5. Integrate expression/gesture state and frame sequences into TUI/GUI surfaces
   once approved dependencies and shell integration exist.
