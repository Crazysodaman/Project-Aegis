# Batch 7 — Agent 06 — Voice: File-Based Windows SAPI STT

Status: **DONE. Windows SAPI TTS still live-verified, and file-based Windows
SAPI STT is now live-verified end-to-end (TTS WAV → file recognition →
transcript). Microphone STT is NOT implemented and NOT verified.**

Scope respected: only `crates/aegis-embodiment/src/**`,
`crates/aegis-embodiment/tests/**`, and this document were changed. No
dependencies, no package installs, no model downloads, no microphone access, no
audible playback, no commits, and no destructive git. `D:\SofiaDev\SofiaAdaLyra`
and `sofia_r9_bluehour_REFERENCE_AIR_medium` were not touched.

## 1. Audit (before changes)

- `crates/aegis-embodiment/src/voice.rs` already had the provider-neutral
  `TtsProvider` / `SttProvider` traits, `TtsRequest` / `SttRequest`, the
  argument-vector `CommandTtsProvider` / `CommandSttProvider` adapters, typed
  `VoiceError`, `AudioFormat`, `probe_availability`, and the per-Core
  `VoiceBinding` view.
- Batch 6 added `SapiTtsProvider` (fixed PowerShell script, WAV file output,
  timeout, bounded stdout/stderr) — this TTS path is unchanged and still works.
- Batch 6 `SapiSttProvider` was a stub that always returned
  `SttUnavailable { reason: "... microphone ..." }`.
- Host probe (enumeration only, no mic, no playback):
  `System.Speech.Recognition.SpeechRecognitionEngine::InstalledRecognizers()`
  reported one desktop recognizer: `en-US | MS-1033-80-DESK`. It supports the
  `DictationGrammar`.

## 2. Exact files changed

- `crates/aegis-embodiment/src/voice.rs`
  - New typed errors: `EmptyTranscript`, `SapiTranscriptTooLarge { bytes, max }`.
  - New public constant `MAX_SAPI_TRANSCRIPT_BYTES` (1 MiB).
  - `SapiSttProvider` rewritten from a stub into a real file-based recognizer
    (fields: name, timeout, max transcript bytes; builders `with_timeout`,
    `with_max_transcript_bytes`; accessors; limit validation).
  - New fixed `SAPI_STT_SCRIPT` PowerShell script (compiled in as `const &str`).
  - Split process handling into `wait_bounded_process` (shared wait/timeout/
    bounded-stdout/stderr loop), `run_sapi_process` (TTS spawn mapping) and
    `run_sapi_stt_process` (STT spawn mapping). The TTS path behavior is
    unchanged.
  - Unit tests: replaced the old STT-stub test with
    `sapi_stt_validates_request_format_and_limits`; added
    `live_sapi_stt_round_trip_transcribes_synthesized_wav_when_enabled`.
- `crates/aegis-embodiment/src/lib.rs`
  - Updated module docs (STT is now file-based WAV recognition, not a stub).
  - Re-exported `MAX_SAPI_TRANSCRIPT_BYTES`.
- `crates/aegis-embodiment/tests/voice.rs`
  - Added `stt_transcript_for_core_a_does_not_cross_into_core_b`.
  - Existing per-Core isolation tests (distinct voice configs, isolated sample
    assets, Core A cannot bind Core B's sample) remain.
- `docs/implementation/BATCH7_AGENT06_VOICE.md` (this file).

`src/voice.rs`, `src/remote.rs`, `src/raster.rs`, and the artifact bins were not
broken. No edits were made to `crates/aegis-cli`, workspace files, or other
crates.

## 3. Design — file-based SAPI STT (not microphone)

- Audio bytes for a `SttRequest` (WAV) are written to `<temp>/input.wav`.
- A fixed PowerShell script is written to `<temp>/sapi_recognize.ps1`; the
  transcript output path is `<temp>/transcript.txt`.
- The script path and file paths are passed as an **argument vector** to
  `powershell -NoProfile -ExecutionPolicy Bypass -File <script> -WaveFile <in>
  -OutFile <out>`. Request data is never interpolated into script code and no
  shell string (`cmd /c`) is used.
- The script enumerates installed recognizers, selects the first one, loads a
  `DictationGrammar`, calls `SetInputToWaveFile($WaveFile)` and `Recognize()`,
  then writes `result.Text` (empty if `null`) to the output file as UTF-8 with
  no BOM.
- Execution is bounded by the provider timeout (default 30 s, capped at 5 min),
  stdout is capped at 8 KiB, stderr at 8 KiB, and the transcript file at 1 MiB.
- The request is validated first: empty audio → `EmptyAudio`, oversized audio →
  `AudioTooLarge`, non-WAV declared format → `UnsupportedFormat`, malformed WAV
  → `InvalidAudio` (the same dependency-free RIFF/WAVE validator used for TTS
  output).
- The temp workspace is removed on drop (best effort), so no temp files remain
  on success or on typed failure.
- No audio is played and no microphone is opened; `System.Speech` is used only
  for file input/output.

### Engine selection and failure handling

- The recognizer is selected by the script as the first installed
  `SpeechRecognitionEngine` (host: `en-US MS-1033-80-DESK`). If the recognizer
  list is empty the script throws a clear message.
- Recognizer/assembly/startup failures are mapped to the typed
  `VoiceError::SttUnavailable { reason }` (spawn failure, non-zero PowerShell
  reporting no recognizer / missing `System.Speech`, or empty stderr). A
  non-zero exit not recognized as unavailability maps to `NonZeroExit`.
- Timeout maps to `Timeout`; empty/missing transcript maps to `EmptyTranscript`;
  invalid UTF-8 maps to `InvalidUtf8`; oversized transcript maps to
  `SapiTranscriptTooLarge`.
- The existing `SttProvider` trait and both command adapters
  (`CommandTtsProvider` / `CommandSttProvider`) are unchanged and keep working.

## 4. Tests and counts

`cargo test -p aegis-embodiment` — **75 passed, 0 failed, 1 ignored** (the
ignored test is the pre-existing opt-in artifact upload test):

| Target | Passed |
| --- | --- |
| `src/lib.rs` unit tests | 9 |
| `src/bin/{artifact-fetch,artifact-node,voice_mock}` | 0 |
| `tests/animation.rs` | 6 |
| `tests/artifact_node.rs` | 5 (1 ignored) |
| `tests/bmp.rs` | 12 |
| `tests/embodiment.rs` | 7 |
| `tests/library.rs` | 4 |
| `tests/remote.rs` | 7 |
| `tests/render.rs` | 3 |
| `tests/scene.rs` | 6 |
| `tests/voice.rs` | 16 |
| doc-tests | 0 |

New / relevant voice tests:

- `voice::tests::sapi_stt_validates_request_format_and_limits`
- `voice::tests::live_sapi_stt_round_trip_transcribes_synthesized_wav_when_enabled`
  (skip unless `AEGIS_VOICE_LIVE=1`; skip if the recognizer is unavailable)
- `tests/voice.rs::stt_transcript_for_core_a_does_not_cross_into_core_b`
- `voice::tests::live_sapi_tts_writes_valid_wav_when_enabled` (unchanged)

Clippy:

```
cargo clippy -p aegis-embodiment --all-targets -- -D warnings
```

Result: clean (no warnings, no errors).

## 5. Live evidence (file-based TTS → STT round trip)

Command:

```
$env:AEGIS_VOICE_LIVE='1'
cargo test -p aegis-embodiment live_sapi -- --nocapture
```

Observed:

```
live SAPI TTS produced 84904 bytes of WAV audio
test voice::tests::live_sapi_tts_writes_valid_wav_when_enabled ... ok
live SAPI TTS->STT WAV bytes: 148836, transcript: The quick brown fox jumps over the lazy dog
test voice::tests::live_sapi_stt_round_trip_transcribes_synthesized_wav_when_enabled ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 1.42s
```

Running only the STT round-trip test by name:

```
$env:AEGIS_VOICE_LIVE='1'
cargo test -p aegis-embodiment live_sapi_stt_round_trip_transcribes_synthesized_wav_when_enabled -- --nocapture
```

Observed:

```
live SAPI TTS->STT WAV bytes: 148836, transcript: The quick brown fox jumps over the lazy dog
test voice::tests::live_sapi_stt_round_trip_transcribes_synthesized_wav_when_enabled ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 1.24s
```

Interpretation:

- TTS (`"Aegis voice check."`) still works: **84,904-byte** WAV.
- The round-trip test synthesizes `"the quick brown fox jumps over the lazy dog"`
  to a **148,836-byte** WAV, recognizes that same file, and returns the non-empty
  transcript **`The quick brown fox jumps over the lazy dog`**.
- No microphone is used; the WAV is written to a temp directory, recognized from
  file, and the temp directory is removed. No audio is played.

## 6. What is NOT implemented / NOT claimed

- **Microphone STT is NOT implemented and NOT verified.** There is no mic
  capture, no default audio device usage, and no live microphone test. The SAPI
  STT path only accepts a caller-supplied WAV file.
- Qwen3-TTS is **not installed** and is not claimed.
- Whisper / faster-whisper / whisper.cpp are **not installed** and are not
  claimed.
- No model was downloaded. No GPU path was added or used.
- The pre-existing `probe_availability` remains deliberately conservative and
  reports command-engine availability as unverified; it was not changed to
  claim SAPI. SAPI STT evidence comes from the opt-in live test above.

## 7. Resource notes

- SAPI TTS and SAPI STT are **CPU-only**; no GPU is used.
- **No download** is required: `System.Speech` and the desktop recognizer ship
  with Windows.
- Short-phrase synthesis plus recognition completes in roughly 1–2 s on this
  host, well within the 30 s default timeout.
- No network access, no package installation, no microphone access, and no
  audible playback occur.

## 8. Limitations

- File-based STT is Windows-only; other platforms return
  `VoiceError::SttUnavailable`.
- Recognizer choice is "first installed recognizer"; per-language/topic grammar
  selection is not exposed. A `DictationGrammar` is used (free-form dictation,
  not a custom grammar for higher accuracy).
- Only PCM WAV input is accepted (`AudioFormat::Wav`); MP3/FLAC/Ogg/PCM16 raw
  return `UnsupportedFormat`. The input WAV is validated structurally, not fully
  decoded.
- Recognition accuracy depends on the installed recognizer; the live test makes
  only a loose keyword assertion in addition to the non-empty check.
- TTS output is fixed 22.05 kHz / 16-bit / mono PCM; voice selection/SSML are
  not exposed through `TtsRequest`.

## 9. Next steps

1. Optionally accept a requested language/culture and pick a matching installed
   recognizer in the fixed script (still passing values as args/files only).
2. Optionally expose SAPI voice-name selection for TTS while keeping the
   fixed-script + argument-vector design.
3. Add microphone STT only after explicit owner approval for hardware access;
   gate it behind a separate opt-in flag and keep it labeled unverified until a
   live mic test passes.
4. If Whisper or Qwen3-TTS are ever installed, add adapters behind the existing
   `TtsProvider` / `SttProvider` traits and extend `probe_availability` with
   live verification.
