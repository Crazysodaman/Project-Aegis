# Batch 8 — Agent M5: Embodiment & Dynamic Presentation

Status: **IMPLEMENTED / TESTED**
HEAD: `2bd2513` (all pre-existing uncommitted work preserved; nothing was reset, cleaned, committed, or deployed)

Files changed (only these):

| File | Change |
| --- | --- |
| `crates/aegis-embodiment/src/presentation.rs` | **new** deterministic outfit selection, voice projection, expression mapping, presentation frame |
| `crates/aegis-embodiment/src/lib.rs` | module declaration + re-exports only |
| `crates/aegis-embodiment/tests/presentation.rs` | **new** 9 integration tests |
| `docs/implementation/BATCH8_AGENT05_M5_EMBODIMENT.md` | this document |

No workspace manifest, `Cargo.lock`, CLI, other crate, appearance data, Core identity, completion ledger, or `D:\SofiaDev\SofiaAdaLyra` changes were made. No new dependencies were added. `voice.rs` was **not** modified; the optional SAPI style binding was deliberately left out (see limitations).

## 1. What is real

All decisions are pure, deterministic functions of the persisted store plus optional borrowed context. Nothing is randomized, and no decision mutates appearance, assets, wardrobe, or Core identity.

### `PresentationContext<'a>`

Borrowed, fully optional context:

```rust
pub struct PresentationContext<'a> {
    pub world: Option<&'a WorldSnapshot>,
    pub emotion: Option<&'a EmotionState>,
    pub activity: Option<&'a Activity>,
    pub personality_tags: &'a [String],
    pub preference_tags: &'a [String],
}
```

Builder methods (`with_world`, `with_emotion`, `with_activity`, `with_personality_tags`, `with_preference_tags`) and `Default` are provided. A missing field removes that signal; it is never replaced with fabricated data.

### `select_outfit(store, core_uuid, ctx, limit) -> Result<OutfitDecision, EmbodimentError>`

* Verifies `store.core_uuid() == core_uuid`; otherwise returns `EmbodimentError::OwnershipMismatch`.
* Reads every asset record including soft-deleted ones, then considers only `Wardrobe` kind.
* Hard constraints, in order:
  1. `core-ownership` — asset `core_uuid` must equal the caller Core;
  2. `live-only` — deleted wardrobe assets are rejected;
  3. `wardrobe-kind` — non-wardrobe assets are never considered;
  4. environmental safety from the weather observation:
     * `cold-safety` when feels-like/temperature is `<= 5 °C` and the asset has no warm/coat/jacket/etc. term;
     * `heat-safety` when `>= 28 °C` and the asset has no cool/light/summer/etc. term;
     * `precipitation-safety` when the condition is rain/snow/storm/etc. or humidity is `>= 90 %` and the asset has no waterproof/rain/boot/etc. term.
* Soft score (tags + asset name, case-insensitive) adds weather safety matches, season, time-of-day, activity title/status, emotion dimensions, and personality/preference tag overlaps.
* Deterministic tie-breaks: score descending, lowercased name ascending, asset id ascending.
* `limit` is capped at `MAX_OUTFIT_SELECTION` (8). `limit = 0` selects no assets while still reporting inventory availability.
* Empty or fully rejected inventory returns `available = false`, an empty `asset_ids`, a reason in `rationale`, and structured `rejected` entries. No asset is ever invented.
* `OutfitDecision` carries `schema`, `core_uuid`, `available`, `asset_ids`, `rationale`, applied `constraints`, `rejected`, and `considered`.

### `voice_projection(emotion: Option<&EmotionState>) -> VoiceProjection`

Deterministic provider-neutral mapping to `{ rate_percent, volume_percent, style_label }`:

* bounds: rate `60..=150`, volume `40..=100` (`MIN/MAX_VOICE_*_PERCENT`);
* `None` yields the neutral projection `100 / 100 / "neutral"`;
* style labels: `tense`, `bright`, `calm`, `curious`, `subdued`, `energetic`, `neutral`;
* all inputs are clamped for finiteness/ranges and all arithmetic is rounded deterministically.

### `expression_for_emotion(appearance, emotion) -> Option<String>`

Maps the emotion to an ordered candidate-name list (`happy`, `joy`, `sad`, `angry`, `thinking`, `calm`, `neutral`, …) and returns the **stored** appearance key that case-insensitively matches the first candidate. It only ever returns a name already present in `appearance.expressions`; if nothing matches it returns `None`. Missing emotion still falls back to `neutral`/`calm`/`default`/`rest` if those exist.

### `render_presentation_frame(store, core_uuid, world, emotion) -> Result<PresentationFrame, EmbodimentError>`

* Verifies Core ownership before doing anything.
* Reads the persisted `Appearance` and live assets, chooses an expression with `expression_for_emotion`, and composes an `AvatarScene` with the existing `AvatarScene::compose`.
* Reuses the existing CPU `render_bmp` rasterizer at `256 x 256`; no new renderer was added.
* If the scene has no renderable layers, returns a deterministic descriptor with `bmp = None` instead of inventing art.
* `PresentationFrame` records `schema`, `core_uuid`, `expression`, `scene`, `bmp`, `width`, `height`, and the optional world `season`/`time_of_day`. World data is metadata only; it never edits approved appearance.

## 2. Tests and evidence

Command:

```
cargo test -p aegis-embodiment
```

Result: **105 passed, 0 failed, 1 pre-existing ignored** across the crate. The new `tests/presentation.rs` has **9 passing integration tests**; `presentation.rs` adds **2 passing unit tests**.

Integration coverage (`tests/presentation.rs`):

1. `two_cores_make_different_outfit_decisions` — distinct Cores with different wardrobes choose different assets from identical context; cross-Core call is rejected.
2. `weather_season_and_time_change_the_selection` — cold/rain/winter/evening chooses the coat; hot/summer/midday chooses shorts.
3. `missing_world_and_emotion_are_honest` — no world/emotion still selects deterministically, neutral voice, no expression, and empty inventory is explicitly unavailable.
4. `selections_never_cross_core_boundaries` — ids stay inside the owning store; cross-Core select/render return `OwnershipMismatch`.
5. `hard_safety_and_identity_constraints_are_enforced` — deleted and unsafe assets are rejected with reasons; non-wardrobe assets are not considered; unsafe-only inventory is unavailable.
6. `voice_projection_is_bounded_and_deterministic` — bounds, labels, repeated equality, and neutral default.
7. `expression_selection_uses_existing_names_only` — case-insensitive existing match; no invented name when only `sad` exists.
8. `rendered_presentation_frame_is_deterministic_and_valid` — byte-identical BMP frames with `BM` magic; empty appearance yields `bmp = None`.
9. `outfit_limit_is_honored_and_bounded` — limit honored/capped, zero-selection behavior, repeatable decisions.

Clippy:

```
cargo clippy -p aegis-embodiment --all-targets -- -D warnings
```

Result: **clean**.

## 3. Heuristics and limitations (honest)

* **Tags are the only clothing metadata.** The store has `kind`, `name`, `media_type`, `tags`, and free-form JSON metadata; this module intentionally reads only `kind`/`name`/`tags`. There is no color, fabric, fit, formality, or thermal model. Substring/tag matching can misclassify unusual names (for example a name containing a safety term).
* **Environmental safety is a keyword gate, not a physics or comfort model.** It reasons from optionally present weather temperature/feels-like, condition words, and humidity. A missing weather observation applies no environmental hard gate; season alone is used only for soft scoring.
* **No Core-identity invention or cross-Core mixing.** Every asset is read through a Core-scoped store and re-checked against the caller UUID. Decisions never include foreign assets, and appearance is never rewritten.
* **Activities contribute only title tokens and active status.** There is no calendar, routine, or long-term preference history model.
* **Preferences are caller-supplied tag slices.** No preference store or learning exists; the caller supplies the tags per decision.
* **Expression selection can return `None`.** If the requested emotion class has no matching named expression, the function refuses to invent one. It can also fall back to neutral-ish existing names.
* **Rendering is schematic.** The BMP compositor draws role-colored rectangles and overlays from the existing raster module. It does not decode PNG/JPEG wardrobe or avatar art, rig a mesh, run a GPU, animate lip-sync, or open a native window.
* **Voice projection is not yet consumed by SAPI.** `voice_projection` is provider-neutral data. `voice.rs` was intentionally left unchanged, so the Windows SAPI provider still uses its existing defaults. Adding an additive `rate/volume` style option is a clean next step.
* **No UI, GUI, or native renderer.** The presentation frame is data plus an optional headless BMP.

## 4. Dependency needs

* No new crates were added.
* New code uses only existing crate dependencies: `aegis-contracts` (`world`, `personal`), `serde`, and the existing embodiment modules (`store`, `appearance`, `asset`, `scene`, `raster`, `error`).
* Tests reuse existing dev-dependencies: `aegis-core-install`, `aegis-platform`, `tempfile`.

## 5. Next steps (not implemented here)

1. Consume `VoiceProjection` in `SapiTtsProvider` as an opt-in rate/volume style option, keeping current defaults unchanged.
2. Let the caller optionally pass selected-outfit layers into `render_presentation_frame` without mutating the approved appearance.
3. Add richer structured wardrobe metadata only if the asset contract is extended; do not infer it from names.
4. Wire a real UI/renderer consumer outside this crate; this module deliberately stays headless.
