# Batch 6 — Agent 06 — Live Voice Integration (Windows SAPI TTS)

Status: **TTS live-verified on this Windows host. STT remains unavailable and
is not claimed.**

## 1. Audit (before changes)

- `crates/aegis-embodiment/src/voice.rs` already defined provider-neutral
  `TtsProvider` / `SttProvider` traits, `TtsRequest` / `SttRequest`,
  `CommandTtsProvider` / `CommandSttProvider` (argument-vector command
  adapters), typed `VoiceError`, `AudioFormat`, `probe_availability`, and the
  per-Core `VoiceBinding` view.
- No local TTS/STT engine was installed (no Piper, Qwen3-TTS, Whisper, etc.).
  The existing availability probe correctly reported both capabilities
  unavailable.
- `EmbodimentStore` already enforces per-Core ownership of voice sample assets:
  `set_voice` calls `require_kind` scoped to the store's Core, and
  `validate_voice_binding` re-checks ownership and SHA-256 integrity.
- Windows 10/11 ships `System.Speech` and at least one SAPI voice. Probe on
  this host:
  - `Microsoft David Desktop` — enabled
  - `Microsoft Zira Desktop` — enabled

## 2. Exact files changed

Added/modified only inside the allowed scope:

- `crates/aegis-embodiment/src/voice.rs`
  - New typed errors: `SapiUnavailable`, `SttUnavailable`, `InvalidAudio`,
    `SapiAudioTooLarge`.
  - `SapiTtsProvider` (Windows): writes a fixed PowerShell script, passes the
    UTF-8 text path and output path via an argument vector, runs
    `powershell -NoProfile -ExecutionPolicy Bypass -File <script> -TextFile <in>
    -OutFile <out>` with a timeout and bounded stdout/stderr, then reads and
    validates the WAV file. No audio is played.
  - WAV validator checks the RIFF/WAVE signatures, the declared RIFF size,
    chunk bounds, a non-empty `fmt ` chunk (channels/sample rate), and a
    non-empty `data` chunk.
  - `SapiSttProvider` design/adapter stub: validates request bounds, then
    returns typed `VoiceError::SttUnavailable`. No microphone is opened.
  - Unit tests: WAV validation, request/format/limit validation, STT
    unavailability, and the env-gated live SAPI test.
- `crates/aegis-embodiment/src/lib.rs`
  - Re-exports `SapiTtsProvider`, `SapiSttProvider`,
    `SAPI_TTS_PROVIDER_NAME`, `SAPI_STT_PROVIDER_NAME`,
    `DEFAULT_SAPI_TIMEOUT_MS`, `MAX_SAPI_AUDIO_BYTES`, and updated module docs.
- `crates/aegis-embodiment/tests/voice.rs`
  - Added `tts_request_for_core_a_cannot_use_core_b_voice_sample`
    (per-Core isolation for TTS voice sample assets).
- `docs/implementation/BATCH6_AGENT06_VOICE.md` (this file).

No dependencies were added; workspace `Cargo.toml` and `Cargo.lock` were not
touched. `sofia_r9_bluehour_REFERENCE_AIR_medium` is untouched.

## 3. SAPI TTS design

- Fixed script is compiled into the binary as a `const &str`; request text is
  **never interpolated into script code**.
- Input text is written to `<temp>/input.txt` as UTF-8. Output is written to
  `<temp>/output.wav`. The script path, text path, and output path are passed
  as separate process arguments. `Command::args` receives a vector — no shell
  string, no `cmd /c`.
- Execution is bounded: default 30 s timeout (`DEFAULT_SAPI_TIMEOUT_MS`),
  stdout capped at 8 KiB, stderr capped at 8 KiB, synthesized WAV capped at
  64 MiB (`MAX_SAPI_AUDIO_BYTES`).
- Typed failures: spawn/assembly/voice errors map to `SapiUnavailable`;
  non-zero unrelated exits map to `NonZeroExit`; timeout maps to `Timeout`;
  malformed output maps to `InvalidAudio`; oversized output maps to
  `SapiAudioTooLarge` / `OutputTooLarge`.
- Supports WAV output only; non-WAV requests return `UnsupportedFormat`.
- Temp workspace is removed on drop (best effort).
- Existing `CommandTtsProvider` / `CommandSttProvider` behavior is unchanged.

## 4. STT status — BLOCKED / NOT CLAIMED

- **Live STT was not implemented or verified.**
- `SapiSttProvider` is an adapter stub that always returns
  `VoiceError::SttUnavailable`. It does not open a microphone, capture audio,
  or run a recognizer.
- SAPI recognition additionally requires a microphone and a configured
  recognizer language; none were captured or validated here.
- Whisper / faster-whisper are **not installed**. Qwen3-TTS is **not
  installed**. No model was downloaded.

## 5. Tests and counts

`cargo test -p aegis-embodiment` — **56 passed, 0 failed** across 10 test
binaries (unit + integration + doc-tests):

- `src/lib.rs` unit tests: 8 passed
  (4 new voice SAPI tests, including the env-gated live test)
- `src/bin/voice_mock.rs`: 0
- `tests/animation.rs`: 6
- `tests/embodiment.rs`: 7
- `tests/library.rs`: 4
- `tests/remote.rs`: 7
- `tests/render.rs`: 3
- `tests/scene.rs`: 6
- `tests/voice.rs`: 15 (includes the new Core A/B voice-sample isolation test)
- doc-tests: 0

New tests relevant to this work:

- `voice::tests::wav_validation_accepts_minimal_pcm_and_rejects_bad_headers`
- `voice::tests::sapi_tts_validates_request_format_and_limits`
- `voice::tests::sapi_stt_reports_typed_unavailable_without_capturing_microphone`
- `voice::tests::live_sapi_tts_writes_valid_wav_when_enabled` (skip unless
  `AEGIS_VOICE_LIVE=1`)
- `tests/voice.rs::tts_request_for_core_a_cannot_use_core_b_voice_sample`

Clippy:

```
cargo clippy -p aegis-embodiment --lib --bins -- -D warnings
```

Result: clean (no warnings, no errors).

## 6. Live TTS evidence

Command:

```
$env:AEGIS_VOICE_LIVE='1'
cargo test -p aegis-embodiment live_sapi -- --nocapture
```

Observed output:

```
live SAPI TTS produced 84904 bytes of WAV audio
test voice::tests::live_sapi_tts_writes_valid_wav_when_enabled ... ok
test result: ok. 1 passed; 0 failed
```

The test asserts a non-empty WAV with `RIFF` at offset 0 and `WAVE` at offset
8. Actual WAV size on this host: **84,904 bytes** (≈ 82.9 KiB) for the phrase
`"Aegis voice check."` at 22.05 kHz, 16-bit mono PCM. The WAV was written to a
temp directory and validated; it was **not played**.

The test skips with a clear message when `AEGIS_VOICE_LIVE` is unset and skips
(as `SapiUnavailable`) if `System.Speech`/a SAPI voice is missing, so the suite
does not fail spuriously on hosts without SAPI.

## 7. Resource notes

- SAPI is a **CPU-based** synthesizer. No GPU is used.
- **No download** is required: `System.Speech` ships with Windows and the
  installed voices are local.
- Typical synthesis latency for a short phrase is sub-second to ~1 s; output
  above was produced within the default 30 s timeout.
- No network access, no package installation, and no audible playback occur.

## 8. Limitations

- SAPI TTS is Windows-only; other platforms return `SapiUnavailable`.
- Output is fixed PCM WAV (22.05 kHz, 16-bit, mono). Voice selection and SSML
  are not exposed through `TtsRequest` yet.
- The voice-sample asset id on a `VoiceProfile` is metadata for this provider;
  SAPI does not clone or condition on a reference sample. Per-Core ownership is
  still enforced by `EmbodimentStore` when binding/validating voices.
- STT is blocked (no mic capture, no recognizer verification).
- `SapiSttProvider` intentionally makes no recognition claim; do not treat it
  as live STT.

## 9. Next steps

1. Optionally expose SAPI voice-name selection (`-VoiceName`) while keeping the
   fixed-script + argument-vector design.
2. Add an opt-in live STT path only after a microphone and recognizer are
   provisioned; gate it behind `AEGIS_VOICE_LIVE` and report it as unverified
   until audio is captured.
3. If Qwen3-TTS or Whisper are later installed, add adapters behind the
   existing `TtsProvider` / `SttProvider` traits and extend
   `probe_availability` with live verification.
4. Consider an explicit `AEGIS_SAPI_VOICE` env/config override, validated
   against the installed-voice list.
