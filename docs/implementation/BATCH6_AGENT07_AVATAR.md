# Batch 6 - Agent 07: Actual Avatar Rendering (BMP Pixels)

Status: implemented and verified in `aegis-embodiment` only. This batch produces
**real pixels** as a 24-bit uncompressed BMP via a dependency-free software
rasterizer. It is intentionally simple rectangle/layer compositing, not full
art, GPU rendering, or a windowed GUI.

No approved Core appearance or Core identity was invented or changed. The
synthetic scene used by the live test carries no Core #7 identity and no real
personal identity.

## Files

Added:

- `crates/aegis-embodiment/src/raster.rs` - software rasterizer, BMP encoder,
  BMP writer, clipping helper.
- `crates/aegis-embodiment/tests/bmp.rs` - header/stride/bottom-up/determinism/
  theme/clipping/animation/per-Core/live tests.
- `docs/implementation/BATCH6_AGENT07_AVATAR.md` - this report.

Edited (additive wiring only):

- `crates/aegis-embodiment/src/lib.rs` - declares `pub mod raster;` and
  re-exports `render_bmp`, `render_bmp_at`, `write_bmp`, `fill_rect`, `Rect`,
  and the BMP constants.

No other crate, workspace manifest, `Cargo.lock`, `.gitignore`, CLI, completion
doc, or `D:\SofiaDev\SofiaAdaLyra` content was touched.

## API

- `render_bmp(scene: &AvatarScene, width: u32, height: u32) -> Result<Vec<u8>, EmbodimentError>`
  - Validates the scene, requires at least one layer, and accepts dimensions in
    `MIN_BMP_DIMENSION..=MAX_BMP_DIMENSION` (16..=2048).
  - Background is derived from the Core theme `primary` color.
  - Visible layers are composited in deterministic `z_index`/role/asset order as
    role-colored rectangles: base/body (`primary`), hair (shaded `primary`),
    eyes (`accent`), outfit (`tertiary`), overlay (`accent`/`primary` blend).
  - Active expression adds an accent rectangle; active gesture adds a tertiary
    rectangle.
  - All rectangles are clipped to the canvas.
- `render_bmp_at(scene, width, height, animation: Option<&AnimationState>)`
  - Same as above but applies the animation transform (translate/scale/opacity)
    so distinct animation times produce distinct pixels.
- `write_bmp(path, bytes) -> Result<(), EmbodimentError>`
  - Enforces `MAX_BMP_BYTES` (64 MiB), `BM` magic, minimum header length, and
    that the declared file size equals the actual byte length before writing.
- `fill_rect(pixels, width, height, Rect, [u8; 3])`
  - Public, bounds-checked clipped rectangle fill used by the rasterizer and the
    clipping test.

BMP layout: 14-byte file header + 40-byte `BITMAPINFOHEADER`, BI_RGB, 24 bpp,
bottom-up rows, BGR pixel order, 4-byte row stride with zero padding.

## What is real

- Actual RGB pixels are computed on the CPU and stored in a real image file.
- Valid 24-bit uncompressed BMP headers, row stride/padding, and bottom-up rows.
- Byte-for-byte deterministic output for identical scene + dimensions.
- Per-theme differences (two Core themes render different pixels).
- Per-Core isolation: scenes composed from each Core only reference that Core's
  assets; foreign/wrong-kind references are rejected at `AvatarScene::compose`.
- Animation frames: the same scene rendered at two animation times differs.
- Expression/gesture overlays visibly change pixels.

## What is NOT implemented (honest scope)

- No GPU rendering (no `wgpu`, Vulkan, D3D, Metal, OpenGL).
- No native window / GUI integration (no `eframe`, `winit`, `winit`-based shell).
- No image decoding or compositing of PNG/JPEG assets: asset bytes are validated
  metadata, not decoded art. Layer rectangles are schematic.
- No real avatar artistry, skinning, rigging, or high-quality rasterization
  (anti-aliasing, gradients, transparency, fonts).
- No lip-sync, visemes, audio-reactive animation, or video encoding.
- No HTML/TUI embedding of the BMP output and no Core #7 appearance.

## Dependencies required for real integration (owner approval needed)

These would be **new dependencies** and are not installed by this batch:

- Image decoding/compositing (PNG/JPEG): e.g. `image`, `png`, or `resvg`.
- GPU renderer: e.g. `wgpu`.
- Native window/GUI shell: e.g. `winit` + `wgpu`/`egui`/`eframe`.
- Video/animation export (optional): e.g. an encoder crate.

Until the owner approves them, the only pixel path is this dependency-free BMP
rasterizer.

## Verification

Commands run (package-scoped only; no whole-workspace gates):

```powershell
cargo test -p aegis-embodiment
cargo clippy -p aegis-embodiment --all-targets -- -D warnings
$env:AEGIS_AVATAR_LIVE='1'; cargo test -p aegis-embodiment live_avatar -- --nocapture
```

Results:

- `cargo test -p aegis-embodiment`: **68 passed, 0 failed** (includes 12 new
  `tests/bmp.rs` tests; 0 failed across all targets/doc-tests).
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings`: clean.
- Live test (`live_avatar_writes_bmp_file`): **1 passed**, 11 filtered out.
  - Synthetic scene, 128x160, default approved Aegis theme, no Core #7 identity.
  - File: `synthetic-avatar.bmp`
  - Bytes: **61494**
  - SHA-256: `3b9c9fd3d9517f8835cc107b49136b4d150e0cdea700dc1e3b3e38fd54629c5f`

New test coverage:

1. BMP `BM` magic, declared file size equals actual, width/height/pixel format,
   pixel-data offset, DIB size, image-size field.
2. Bottom-up row ordering (top row background vs bottom row base color).
3. Four-byte row stride and zero padding for non-multiple-of-4 widths.
4. Byte-for-byte deterministic rendering.
5. Two themes produce different pixels.
6. Expression/gesture overlays change pixels.
7. Animation movement clip renders different pixels at different times.
8. Rectangle clipping at negative and beyond-edge coordinates.
9. Invalid dimensions (below 16, above 2048, zero) and empty scenes rejected.
10. Per-Core themes render differently; foreign assets -> `OwnershipMismatch`;
    wrong asset kind -> `WrongAssetKind`; unresolved asset -> `NotFound`.
11. `write_bmp` writes the file and rejects truncated/mismatched-size input.
12. Live BMP write/read/hash gated by `AEGIS_AVATAR_LIVE=1`.

## Limitations

- Rects are schematic and fixed per role; there is no asset-driven silhouette.
- Transparency is approximated by blending toward the theme background; there is
  no alpha channel (BMP is 24-bit).
- No anti-aliasing; edges are hard pixel steps.
- Animation only moves/scales/blends the same rectangles; no skeletal motion.
- The BMP writer validates structure but is not a general image library.

## Next steps (pending owner approval)

1. Decide whether image decoding/compositing may be added (`image`/`png`).
2. If windowed preview is required, approve a GPU/window stack (`wgpu` +
   `winit`/`eframe`) and add it behind a feature flag.
3. Optionally add an alpha-capable output (32-bit BMP/PNG) once a decoder/encoder
   is approved.
4. Integrate the raster output into TUI/GUI surfaces once approved dependencies
   and shell integration exist.
