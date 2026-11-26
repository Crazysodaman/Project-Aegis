# Aegis FINAL - Agent 08 (Voice & Speech) Report

Status: **Complete**
Baseline HEAD: `2bd2513`
Scope: connect the existing presentation `VoiceProjection` to Windows SAPI TTS
through an additive, bounded, optional rate/volume style. Defaults are unchanged.

No new dependencies. No audio playback and no microphone capture. `lib.rs`,
other embodiment modules, other crates, the CLI, and workspace files were not
edited.

## Files changed

- `crates/aegis-embodiment/src/voice.rs` - add `TtsStyle`, style bounds/errors,
  optional `TtsRequest::style`, provider default style, SAPI argument builder,
  and PowerShell `-Rate`/`-Volume` parameters.
- `crates/aegis-embodiment/tests/voice.rs` - add 4 integration tests (defaults,
  projection derivation, bounds rejection, live non-default-rate WAV).
- `docs/implementation/AEGIS_FINAL_AGENT08_REPORT.md` (this file).

`VoiceProjection` remains in `presentation.rs` and is imported in `voice.rs`
via `crate::presentation::VoiceProjection`; it was not moved.

## Behavior

- `TtsStyle { rate_percent, volume_percent }` defaults to `100 / 100`, exactly
  the legacy SAPI behavior (SAPI rate `0`, volume `100`).
- Bounds: rate `50..=200` percent, volume `0..=100` percent. Out-of-bounds
  values return typed `VoiceError::InvalidRatePercent` /
  `InvalidVolumePercent`.
- `TtsStyle::from_projection(&VoiceProjection)` and
  `TtsRequest::with_projection` / `SapiTtsProvider::with_projection` connect
  presentation emotion output to synthesis.
- A request style overrides a provider default style. No style means defaults.
- Rate percent maps deterministically to SAPI `-10..=10` (`100% -> 0`, `150% -> 5`,
  `60% -> -4`).
- PowerShell receives all paths and numeric values as separate argument-vector
  entries (`-File`, `-TextFile`, `-OutFile`, `-Rate`, `-Volume`); no shell string
  is built and text is still read from a UTF-8 file.
- The command TTS/STT providers and the existing SAPI STT provider are
  unchanged. The additive style is consumed only by `SapiTtsProvider`.

## Verification

- `cargo test -p aegis-embodiment`: **115 passed, 1 ignored, 0 failed**.
  - `tests/voice.rs`: **20 passed** (16 preserved + 4 new).
  - `src/voice.rs` unit tests: **8 passed** (5 preserved + 3 new).
  - Existing TTS, STT, projection, batch8 QA, and Sofia reference tests remain
    green.
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings`: **clean**.
- Live evidence (`AEGIS_VOICE_LIVE=1`):
  - `live_sapi_tts_synthesizes_nondefault_rate_when_enabled` passed.
  - Requested rate `80%`, volume `70%`; SAPI produced a valid RIFF/WAVE file of
    **184,334 bytes** with no playback.

## Limitations

- SAPI TTS is Windows-only; non-Windows returns `SapiUnavailable`.
- The live test is env-gated by `AEGIS_VOICE_LIVE=1` and skips cleanly when SAPI
  is unavailable; the default test run does not touch audio.
- Style is bounded percentage metadata mapped to SAPI's coarse `-10..=10` rate
  and `0..=100` volume; it is not a full SSML/prosody model and does not select
  a specific installed voice from `VoiceProfile`.
- Because `lib.rs` was intentionally left untouched, the new `TtsStyle` and
  bound constants are reachable through `aegis_embodiment::voice::*` rather than
  the crate root.
- No audio playback, microphone access, lip-sync, or new media formats were
  added.
