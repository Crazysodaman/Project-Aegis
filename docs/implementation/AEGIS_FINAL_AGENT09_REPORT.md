# Aegis Final — Agent 09 (Avatar, Expression & Animation)

Scope: `crates/aegis-embodiment` only, HEAD `2bd2513`. No other crates, Core
appearances, assets, CLI, workspace, or `D:\SofiaDev\SofiaAdaLyra` were touched.
No new dependencies, no unsafe, no GPU/window install, no commit/reset/clean.

## Files

- `crates/aegis-embodiment/src/animation.rs` — added deterministic
  `AvatarState`, `MotionMode`, `StateSignal`, `project_avatar_state()`, and
  `AvatarState::clip()` (generated idle/ambient/active/resting loop).
- `crates/aegis-embodiment/src/expression.rs` — added `StateOverlay` and
  `render_state_bmp()`; expression, gesture, and lip-sync mouth overlays painted
  on an animation-transformed base BMP. Existing `render_expression_bmp()`
  behavior preserved.
- `crates/aegis-embodiment/src/sequence.rs` — added `SequenceOptions`,
  `render_state_sequence()`, `write_state_sequence[_capped]()`, and bounded
  manifest fields `avatar_state` / `mouth_frames` (serde-skipped when absent,
  so existing manifests are unchanged).
- `crates/aegis-embodiment/src/lipsync.rs` — documentation now states the plan is
  a text/timing heuristic, not audio-synced, and that `render_state_sequence()`
  consumes it.
- `crates/aegis-embodiment/tests/avatar_state.rs` — 7 new tests.
- `crates/aegis-embodiment/tests/avatar_state_live.rs` — 1 env-gated live test.
- `docs/implementation/AEGIS_FINAL_AGENT09_REPORT.md` — this report.

Projection is total and deterministic: emotion dimensions choose
expression/gesture/energy; activity and optional world time-of-day/season/weather
adjust motion/energy. Missing emotion keeps `Neutral` + `Gesture::None`; missing
world/activity are simply absent from `signals`, never invented. If an emotion
carries a Core UUID, `render_state_sequence()` enforces it against
`scene.core_uuid` and returns `OwnershipMismatch` on a cross-Core state.

## Verification

- `cargo test -p aegis-embodiment`: **123 passed, 0 failed, 1 ignored**
  (ignored test is the pre-existing LAN/artifact node test). New tests: 7 in
  `avatar_state.rs` + 1 live test in `avatar_state_live.rs`.
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings`: **clean**.

Coverage includes: deterministic projection; honest missing-signal degradation;
two Cores with different emotion producing different frame SHAs; multi-frame
differing hashes; lip-sync mouth frames changing pixels; mismatched
fps/duration/frame-count rejection; manifest state + mouth recording; oversized
write rejection before files are created; scene immutability; Core ownership and
asset-boundary checks.

## Live frames evidence (`AEGIS_AVATAR_LIVE=1`)

Ran `cargo test -p aegis-embodiment --test avatar_state_live -- --nocapture`.
Options: 12 fps, 1500 ms, 128x160, transcript plan over all 18 frames.

- Core A (`...aa`), `happy` / `thumbs_up` / `active`: frames=18,
  bytes=1,106,892, unique_hashes=10.
  - frame 0: 61,494 bytes, SHA-256 `bf85b8de28c138c212b3a701d1cd3124606962dfc3c021c4b0cd953a161ee367`
  - frame 9: 61,494 bytes, SHA-256 `0842b29233b431236e7779979cf138690253c3ee2c0e2f24353a6bd875a4d470`
  - frame 17: 61,494 bytes, SHA-256 `1fd1ea0d47a4530e321c112e8e816ec89cc4e48372c94ef5632f54a1214487a7`
- Core B (`...bb`), `angry` / `shrug` / `idle`: frames=18,
  bytes=1,106,892, unique_hashes=5.
  - frame 0: 61,494 bytes, SHA-256 `fc83d42ad4b01933bbfdc84daaf61f19752dbf53322d6699e0b2eb572896627b`
  - frame 9: 61,494 bytes, SHA-256 `1b35e45dec518317942702ebad93e6bf0db3879a4345d3a3cff793a0a3fa89c5`
  - frame 17: 61,494 bytes, SHA-256 `1b35e45dec518317942702ebad93e6bf0db3879a4345d3a3cff793a0a3fa89c5`
- Both wrote `manifest.json` with `aegis.embodiment.avatar_state.v1` and 18
  mouth frames; frame-0 hashes differ across Cores. Equal byte totals are
  expected because BMP size depends on dimensions/frame count, not content.

## Real vs. not real

Real: deterministic BMP byte output, per-frame SHA-256, manifests, multi-frame
movement, state-driven expression/gesture/mouth overlays, bounded writes, and
lip-sync mouth-frame integration from text + optional caller timings.

Not real: no native/GPU renderer, no window, no decoded art (PNG/JPEG), no
rigging, no audio playback/analysis, and no real phoneme/viseme lip-sync. The
renderer is schematic CPU rectangle compositing; lip-sync is a documented text
heuristic, not audio-synced.

## Limitations

- Colors/geometry are synthetic placeholders, not approved Core art.
- Mouth shapes are a 4-value schematic cycle, not visemes; timings are optional
  caller input or even distribution.
- `AvatarState` projection is a small fixed priority heuristic over the six
  emotion dimensions plus simple activity/world modifiers.
- New types are exposed through module paths (`aegis_embodiment::animation`,
  `::expression`, `::sequence`) because `lib.rs` was out of scope for this task.
