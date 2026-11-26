# Batch 5 — Agent 07: Avatar, Rendering & Animation Infrastructure

Status: implemented and verified (`aegis-embodiment` only). No pixel, GPU, or
native-window rendering was performed.

## Audit (before this batch)

`aegis-embodiment` already had:

- `Appearance` (`appearance.rs`): schema, Core UUID, optional avatar asset,
  theme colors, named expression/gesture bindings, wardrobe asset ids, optional
  voice profile, revision, and validation (`schema`, identity, bounds, hex
  colors, animation names).
- `Asset` / `AssetKind` (`asset.rs`): avatar, expression, gesture, wardrobe,
  voice sample, theme, texture, other, with SHA-256 metadata.
- `EmbodimentStore` (`store.rs`): per-Core ownership, asset-kind checks
  (`require_kind`), soft delete, and appearance persistence.
- Provider-neutral voice contracts (`voice.rs`) and tests.

There was **no** scene model, layer model, animation clip, keyframe, easing,
animation player, renderer trait, scene description, or deterministic headless
path.

No approved Core appearance/identity was changed or inferred. The scene model
consumes only the persisted `Appearance` and asset metadata.

## Implemented (additive, tested)

### 1. Renderer-agnostic scene model — `src/scene.rs`

- `LayerRole { Base, Hair, Eyes, Outfit, Overlay }` with conventional z-order.
- `SceneLayer { role, asset_id, asset_kind, media_type, sha256, visible, z_index }`.
- `ActiveMotion { name, asset_id }` for the active expression/gesture.
- `AvatarScene { schema, core_uuid, theme, layers, expression, gesture }`.
- `AvatarScene::new(...)` validates theme, Core UUID, layer count
  (`MAX_SCENE_LAYERS = 64`), references, and role cardinality (at most one base,
  hair, eyes, overlay; multiple outfit layers allowed), and sorts layers
  deterministically.
- `AvatarScene::compose(appearance, assets, expression, gesture)` builds a scene
  from persisted appearance + resolved assets:
  - base layer from `avatar_asset_id` (must be `AssetKind::Avatar`);
  - outfit layers from `wardrobe` (each must be `AssetKind::Wardrobe`);
  - active expression/gesture resolved through `expressions` / `gestures`
    maps and checked as `Expression` / `Gesture`.
- Every referenced asset must be present, owned by the appearance Core, live, and
  of the expected kind. Foreign-Core references return `OwnershipMismatch`;
  wrong kinds return `WrongAssetKind`; missing references return `NotFound`.
- Hair/eyes/overlay are part of the model for renderers/callers, but are not
  auto-inferred from tags or invented configuration.

### 2. Deterministic animation model — `src/animation.rs`

- `Transform { translate_x, translate_y, scale, rotation_deg, opacity }`.
- `Keyframe { at_ms, transform, easing }`.
- `Easing { Linear, EaseIn, EaseOut, EaseInOut, Step }` with clamped `apply`.
- `PlaybackMode { Once, Loop }`.
- `AnimationClip::new(name, duration_ms, playback, keyframes)` sorts keyframes and
  rejects empty/oversized (`MAX_KEYFRAMES = 1024`) sets, zero duration, duplicate
  times, times beyond duration, invalid names, and non-finite transforms.
- `AnimationClip::sample(time_ms)` interpolates/clamps deterministically.
- `AnimationPlayer` is pure: `advance(elapsed_ms)` accumulates caller-supplied
  time and `state()` reports `time_ms`, `total_elapsed_ms`, `finished`, and
  `transform`. `Once` clamps at duration and sets `finished`; `Loop` wraps modulo
  duration and never finishes. `reset()` and `seek()` are provided.

### 3. Renderer contract + headless renderer — `src/render.rs`

- `SceneRenderer` trait: `name()` and
  `render(scene, animation_state) -> SceneDescription`.
- `AvatarRenderer` marker sub-trait.
- `HeadlessSceneRenderer` implements both and emits a deterministic,
  serializable `SceneDescription` with:
  - description schema and source scene schema, renderer name, Core UUID, theme;
  - deterministically sorted layers;
  - active expression/gesture references;
  - optional animation state;
  - explicit `rasterized: false`, `gpu_used: false`, `windowed: false`.
- `SceneDescription::to_json()` / `to_json_pretty()` produce identical output for
  identical input (no timestamps, randomness, or hash-map iteration).

### 4. Per-Core isolation

- Composition and rendering are bound to the scene's Core UUID.
- Cross-Core asset references are rejected at compose time with
  `OwnershipMismatch`; wrong asset kinds are rejected with `WrongAssetKind`.
- Tests build and render scenes for two Cores with different themes, avatars, and
  wardrobes and assert no asset/theme/description contamination.

## Files changed

- `crates/aegis-embodiment/src/scene.rs` (new)
- `crates/aegis-embodiment/src/animation.rs` (new)
- `crates/aegis-embodiment/src/render.rs` (new)
- `crates/aegis-embodiment/src/lib.rs` (additive modules, docs, re-exports)
- `crates/aegis-embodiment/tests/scene.rs` (new)
- `crates/aegis-embodiment/tests/animation.rs` (new)
- `crates/aegis-embodiment/tests/render.rs` (new)
- `docs/implementation/BATCH5_AGENT07_AVATAR.md` (new)

No other crate, workspace manifest, lockfile, `.gitignore`, `docs/completion`, or
`D:\SofiaDev\SofiaAdaLyra` content was modified. No dependencies were added.

## Tests and counts

`cargo test -p aegis-embodiment` — **40 passed, 0 failed** (25 existing + 15 new):

- `tests/embodiment.rs`: 7 passed (existing)
- `tests/library.rs`: 4 passed (existing)
- `tests/voice.rs`: 14 passed (existing)
- `tests/scene.rs`: **6 passed** (new)
- `tests/animation.rs`: **6 passed** (new)
- `tests/render.rs`: **3 passed** (new)

New tests:

`tests/scene.rs`

1. `compose_builds_layers_theme_and_active_motion`
2. `default_appearance_composes_an_empty_scene`
3. `compose_rejects_wrong_asset_kind`
4. `compose_rejects_assets_owned_by_another_core`
5. `scene_constructor_validates_structure`
6. `two_cores_compose_isolated_scenes`

`tests/animation.rs`

1. `once_clamps_and_reports_finished`
2. `loop_wraps_time_and_never_finishes`
3. `interpolation_honors_easing`
4. `sampling_clamps_before_first_and_after_last_keyframe`
5. `reset_and_seek_recompute_state`
6. `invalid_clips_are_rejected`

`tests/render.rs`

1. `headless_render_is_deterministic_and_explicitly_non_pixel`
2. `headless_render_without_animation_has_no_pixels_and_empty_layers`
3. `render_preserves_per_core_boundaries`

`cargo clippy -p aegis-embodiment --all-targets -- -D warnings` — **clean**.

## What is real vs NOT implemented

Real in this batch:

- Validated scene composition from existing appearance and asset metadata.
- Strict per-Core asset ownership and asset-kind validation at scene level.
- Pure, deterministic animation timing, interpolation, easing, looping, and
  clamping.
- Deterministic, serializable headless scene descriptions suitable for a future
  renderer, GUI, or test harness to consume.

**NOT implemented in this batch:**

- **No rasterization or pixels.** No image is decoded, composited, or written.
- **No GPU work, shaders, or graphics API.**
- **No native window or event loop.**
- **No lip-sync, visemes, audio-driven animation, or TTS synchronization.**
- No skeletal animation, morph targets, blend trees, physics, or particle
  systems.
- No media payload validation: `media_type`/`sha256` are references copied from
  the asset record; this module does not decode or verify bytes itself.
- Hair/eye/overlay layers are supported by the model but not auto-populated from
  current appearance config (there are no such fields to read).
- Expression/gesture assets are referenced as active motions, not interpreted as
  frames or parameters.
- Animation clips are in-memory values; no clip persistence, timeline editor, or
  transport format is added.
- No wall-clock source: the caller supplies elapsed milliseconds.

No rendered image, video, or native window was produced or claimed.

## Next steps and dependency requirements

1. **Image decoding/rasterization.** Requires approval for an image crate (for
   example `image`) plus licensing/security review. Decode avatar/outfit assets
   through `EmbodimentStore::read_asset` (SHA-256 verified) and composite layers
   by `z_index`; keep the headless path as the deterministic reference test.
2. **GPU backend.** Requires a graphics dependency (for example `wgpu`) and GPU
   adapter/hardware validation. Upload decoded textures, implement the shader
   pipeline, and add environment-gated GPU tests; continue to emit the headless
   description for golden comparisons.
3. **Native window.** Prefer the existing `aegis-ui` stack rather than adding a
   second windowing dependency. Only introduce a new windowing crate if
   `aegis-ui` cannot host the avatar surface.
4. **Lip-sync.** Requires real TTS output from the Agent 06 command provider plus
   a viseme/phoneme mapping. Add an audio timeline that drives mouth-layer
   animation channels; no TTS or audio engine is installed or verified here.
5. **Animation persistence/authoring.** Define a clip schema (JSON) and store it
   as a Core-owned asset (likely `AssetKind::Other` with a JSON media type to
   avoid a schema migration), then add authoring/import tools.
6. **Frame loop integration.** Add an application-layer wall-clock adapter that
   calls `AnimationPlayer::advance` at a fixed cadence; keep the player pure and
   deterministic for tests.
7. **Media validation.** Add optional magic-byte/header checks for image assets
   before a renderer consumes them, without changing the existing metadata-only
   contract.
8. **Environment-gated visual tests.** Add real render tests (screenshots or GPU
   readback) only after a renderer exists, and keep pixel-free golden tests as
   the portable baseline.

## Limitations

- The scene description is a data contract, not a draw command stream; a real
  backend must interpret layer references itself.
- Asset paths are intentionally absent from the description; a renderer must
  resolve Core-owned assets through the store to retain path containment and
  SHA-256 verification.
- The model does not enforce visual correctness (for example, layer overlap,
  anchors, or pivots); it enforces identity, ownership, kind, and structural
  bounds only.
- Timing is integer-millisecond and floating-point interpolation; extremely long
  playback relies on saturating accumulation.
- No performance or memory claims are made for future raster/GPU rendering.
