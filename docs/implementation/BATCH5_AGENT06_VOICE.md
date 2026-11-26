# Batch 5 — Agent 06: Provider-Neutral Voice System

Status: implemented and verified (`aegis-embodiment` only); no live audio.

## Audit (before this batch)

- `aegis-embodiment` stored a `VoiceProfile` (`voice_id`, `display_name`,
  optional `sample_asset_id`) inside `Appearance` and validated the sample
  asset kind through `EmbodimentStore::set_voice`.
- There was **no** TTS provider, STT provider, command adapter, audio format
  type, request/result/error contract, availability probe, or Core-bound voice
  validation.
- **No real TTS/STT engine is available in this environment.** Verified with
  `Get-Command`:
  - `whisper`, `piper`, `faster-whisper`: **not installed**.
  - `ollama`: installed, but it serves LLM inference only (no bundled TTS/STT
    model).
  - `ffmpeg`: installed, but it is a media tool, not a synthesis or recognition
    engine.
- The approved Sofía reference `sofia_r9_bluehour_REFERENCE_AIR_medium`
  (`aegis-core-profiles::SOFIA_VOICE_REFERENCE`) was **not touched or
  replaced**. This batch does not modify `aegis-core-profiles`.

## Implemented (additive)

### 1. Provider-neutral contracts (`src/voice.rs`)

- `TtsProvider`: `text -> TtsResult { audio, format, media_type, provider,
  voice_id }`.
- `SttProvider`: `audio + AudioFormat -> SttResult { transcript, format,
  provider, language }`.
- `TtsRequest` / `SttRequest` with validation:
  - non-empty TTS text (`trim`), max `MAX_TTS_TEXT_BYTES` (16 KiB);
  - non-empty STT audio, max `MAX_STT_AUDIO_BYTES` (64 MiB);
  - optional language tag validated for emptiness, control characters, and
    `MAX_VOICE_LANGUAGE_CHARS` (32);
  - optional `VoiceProfile` validated (non-empty id/display name, bounded
    sample id).
- `AudioFormat` closed set, documented as **metadata only**: `wav`, `mp3`,
  `flac`, `ogg`, `pcm16le`, each with media type and extension. No bytes are
  decoded or audio-validated.
- `VoiceError` typed errors: empty/oversized text or audio, invalid language or
  voice profile, unsupported format, empty command, argument-vector bounds,
  invalid timeout/output limit, spawn failure, I/O, timeout, non-zero exit,
  oversized output, empty output, invalid UTF-8.

### 2. Owner-configured command adapters

- `CommandSpec { program, args, timeout, max_output_bytes }`:
  - arguments are stored and passed as a `Vec<String>` / `args()` slice, never
    interpolated into a shell command string;
  - max 32 arguments, max 1024 bytes per argument, no NUL bytes;
  - timeout bounded to 1 ms–300 s (default 30 s);
  - output cap bounded to 1 byte–64 MiB (default 32 MiB).
- `CommandTtsProvider` sends request text on the child's stdin and returns the
  child's stdout as audio bytes.
- `CommandSttProvider` sends request audio on the child's stdin and returns the
  child's stdout as a UTF-8 transcript.
- Both poll `try_wait` against a deadline, kill and reap on timeout, drain
  stdout/stderr on background threads to avoid pipe deadlocks, cap stdout at the
  configured limit, cap stderr at 8 KiB for error messages, and map every
  failure to a typed `VoiceError`.

### 3. Per-Core voice binding preservation

- Reuses the existing `VoiceProfile`; no schema or storage change.
- `EmbodimentStore::validate_voice_binding()` / `voice_binding()` returns
  `Option<VoiceBinding>` and, when a sample is referenced:
  - resolves it through the store's Core-scoped `require_kind`, so another
    Core's sample is `NotFound`;
  - rejects deleted samples and wrong asset kinds;
  - rejects an asset whose `core_uuid` does not match this store (defensive
    `OwnershipMismatch`);
  - re-reads the sample through `read_asset`, so SHA-256 integrity and path
    containment are enforced.
- `set_voice` was already Core-scoped; it now documents that a foreign sample
  can never be bound.

### 4. Availability probe

- `probe_availability() -> VoiceAvailability` reports TTS and STT as
  **unavailable** and **not verified**, with reasons.
- It searches `PATH` for recognized executables
  (`piper`/`piper-tts`/`qwen3-tts`; `whisper`/`faster-whisper`/`whisper-cli`/
  `whisper-ctranslate2`).
- **Configuration is never treated as working audio:** `available` stays
  `false` even if an executable is detected, and the reason states that live
  audio was not verified. In this environment no recognized executable exists,
  so the reason also states that no local engine was found and that Ollama has
  no TTS/STT model.

## Files changed

- `crates/aegis-embodiment/src/voice.rs` (new)
- `crates/aegis-embodiment/src/bin/voice_mock.rs` (new; test-only executable)
- `crates/aegis-embodiment/src/lib.rs` (module + re-exports)
- `crates/aegis-embodiment/src/store.rs` (`validate_voice_binding` /
  `voice_binding`)
- `crates/aegis-embodiment/tests/voice.rs` (new)
- `docs/implementation/BATCH5_AGENT06_VOICE.md` (new)

No other crate, workspace manifest, lockfile, `.gitignore`, or
`docs/completion` file was modified.

## Tests and counts

`cargo test -p aegis-embodiment` — **25 passed, 0 failed**:

- `tests/embodiment.rs`: 7 passed (unchanged)
- `tests/library.rs`: 4 passed (Agent 08, unchanged)
- `tests/voice.rs`: **14 passed** (new)

New tests:

1. `tts_request_validation_rejects_empty_and_oversized_text`
2. `tts_request_validation_rejects_invalid_language_and_voice`
3. `stt_request_validation_rejects_empty_and_oversized_audio`
4. `command_spec_validation_enforces_bounds`
5. `command_tts_provider_round_trips_through_mock_executable`
6. `command_stt_provider_round_trips_through_mock_executable`
7. `command_provider_maps_timeout`
8. `command_provider_maps_nonzero_exit`
9. `command_provider_enforces_output_cap`
10. `command_tts_provider_rejects_empty_output`
11. `availability_probe_reports_unavailable_with_reasons`
12. `voice_binding_is_absent_when_unset`
13. `cores_keep_distinct_voice_configs_and_isolated_samples`
14. `voice_binding_rejects_wrong_kind_and_detects_tampering`

The command-adapter tests spawn a trivial local mock executable
(`voice_mock`) that echoes stdin to stdout, sleeps, fails, or emits a bounded
amount of output. They require no audio hardware and never perform synthesis or
recognition.

`cargo clippy -p aegis-embodiment --all-targets -- -D warnings` — **clean**.

## Hardware / model requirements (not installed here)

These are vendor/community planning figures as of 2026, not measured in this
environment.

### Qwen3-TTS (Alibaba Qwen; Apache-2.0)

- Checkpoints: 0.6B and 1.7B (Base / CustomVoice / VoiceDesign) plus a ~0.7 GB
  12 Hz speech tokenizer.
- 1.7B weights are ~4.5 GB on disk; community measurements report roughly
  4–8 GB VRAM in bf16 depending on runtime, activations, and KV cache. An 8 GB
  NVIDIA GPU is the practical floor; 12 GB+ is comfortable.
- Official examples are CUDA/PyTorch-oriented (`device_map="cuda:0"`,
  `torch.bfloat16`, optional FlashAttention-2). CPU batch inference is possible
  but not real-time.
- Software: Python 3.10–3.12, CUDA-enabled PyTorch, `qwen-tts`, `ffmpeg` for
  container handling.
- Licensing permits commercial use; this is noted for planning only.

### Whisper large-v3-turbo (OpenAI; MIT)

- 809M parameters (large-v3 decoder pruned from 32 to 4 layers); weights are
  ~1.6 GB fp16 on disk.
- OpenAI reference implementation states ~6 GB VRAM. With
  `faster-whisper`/CTranslate2, measured footprints are lower (~4–5 GB fp16,
  ~1–4 GB int8); `whisper.cpp` quantized builds are smaller still.
- Runs on 8 GB GPUs comfortably; CPU-only works but is slower.
- Software: `faster-whisper` + CTranslate2 (preferred for this adapter), or
  `whisper.cpp` for a portable binary.
- Note: turbo is optimized for transcription, not translation.

An Aegis host that wants both at once should budget the sum of TTS and STT
footprints plus the existing LLM and display headroom; a 12–16 GB GPU is a
reasonable planning target, not a verified requirement.

## Explicit statement on live audio

**No live TTS or STT was performed.** No model was downloaded or installed, no
microphone was opened, no speaker was driven, and no audio file was synthesized
or transcribed. All provider tests used the non-audio `voice_mock` executable.
The availability probe is deliberately conservative and reports both
capabilities as unavailable and unverified in this environment.

## Limitations

- Formats are declared metadata; payload bytes are not decoded or validated
  against the declared format.
- No microphone capture, playback device, VAD, resampling, streaming,
  interruption/STOP, or lip-sync.
- Command adapters inherit the process environment; owners must configure only
  trusted local executables.
- The STT adapter assumes a UTF-8 transcript on stdout; empty transcripts are
  allowed (silence), TTS empty output is rejected.
- Per-Core binding is validated at rest/read time but is not yet enforced by a
  live audio output path (there is none).
- No cross-host voice configuration sync.
- `voice_mock` is a test helper shipped with the crate's targets; it is not a
  real provider and must not be configured as one.

## Next steps

1. In an owner-approved environment, install and pin Qwen3-TTS and
   `faster-whisper`, then wrap each in a small local command adapter that reads
   stdin and writes stdout.
2. Add optional container header validation (WAV/FLAC/Ogg/MP3 magic bytes)
   before returning results, without decoding audio.
3. Add a Windows audio capture/playback backend (WASAPI) and wire TTS/STT into
   the conversation loop with explicit STOP/interruption.
4. Add live, environment-gated integration tests (real audio hardware and
   models) and only then let the availability probe report `available = true`.
5. Verify per-Core voice isolation end-to-end using
   `sofia_r9_bluehour_REFERENCE_AIR_medium` as the preserved reference without
   overwriting it.
6. Benchmark latency, memory, and concurrency on the target host before
   enabling voice in any release.
