//! Provider-neutral voice contracts: text-to-speech (TTS) and speech-to-text
//! (STT).
//!
//! These interfaces describe *how* a caller hands text or audio to a provider
//! and receives audio or a transcript. They do **not** implement, download, or
//! claim that any synthesis or recognition engine works. A provider is only as
//! real as the owner-configured executable behind it, and that executable must
//! be verified separately with live audio.
//!
//! # Supported audio formats
//!
//! [`AudioFormat`] is a closed set of container/encoding labels used as
//! metadata only: WAV, MP3, FLAC, Ogg Vorbis, and raw signed 16-bit
//! little-endian PCM. No bytes are decoded or audio-validated here; a provider
//! declares the format it produces or accepts. Callers must not assume the
//! declared format was verified by decoding.
//!
//! # Command providers
//!
//! [`CommandTtsProvider`] and [`CommandSttProvider`] run an owner-configured
//! local executable. Fixed arguments are passed as an argument vector (never a
//! shell command string), and the request payload is written to the child's
//! stdin. The response payload is read from the child's stdout, subject to a
//! timeout and a hard output-size cap. Missing executables, non-zero exits,
//! timeouts, oversized output, and invalid UTF-8 are reported as typed
//! [`VoiceError`] values.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{io::Read, io::Write, thread};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::appearance::VoiceProfile;
use crate::presentation::VoiceProjection;

/// Schema identifier for a voice availability report.
pub const VOICE_AVAILABILITY_SCHEMA: &str = "aegis.embodiment.voice.availability.v1";
/// Maximum size of TTS input text, in bytes.
pub const MAX_TTS_TEXT_BYTES: usize = 16 * 1024;
/// Maximum size of STT input audio, in bytes (64 MiB).
pub const MAX_STT_AUDIO_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum characters in a BCP-47 style language tag.
pub const MAX_VOICE_LANGUAGE_CHARS: usize = 32;
/// Maximum characters in a voice id or sample asset id.
pub const MAX_VOICE_ID_CHARS: usize = 128;
/// Maximum number of fixed command arguments.
pub const MAX_COMMAND_ARGS: usize = 32;
/// Maximum length of a single fixed command argument, in bytes.
pub const MAX_COMMAND_ARG_CHARS: usize = 1024;
/// Maximum allowed command timeout, in milliseconds (5 minutes).
pub const MAX_COMMAND_TIMEOUT_MS: u64 = 300_000;
/// Maximum allowed command output cap, in bytes (64 MiB).
pub const MAX_COMMAND_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Default command timeout, in milliseconds.
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 30_000;
/// Default command output cap, in bytes (32 MiB).
pub const DEFAULT_COMMAND_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
/// Default TTS speaking rate, in percent of normal.
pub const DEFAULT_TTS_RATE_PERCENT: u16 = 100;
/// Default TTS volume, in percent of full scale.
pub const DEFAULT_TTS_VOLUME_PERCENT: u16 = 100;
/// Lowest accepted TTS speaking rate, in percent of normal.
pub const MIN_TTS_RATE_PERCENT: u16 = 50;
/// Highest accepted TTS speaking rate, in percent of normal.
pub const MAX_TTS_RATE_PERCENT: u16 = 200;
/// Lowest accepted TTS volume, in percent of full scale.
pub const MIN_TTS_VOLUME_PERCENT: u16 = 0;
/// Highest accepted TTS volume, in percent of full scale.
pub const MAX_TTS_VOLUME_PERCENT: u16 = 100;

const MAX_STDERR_BYTES: usize = 8 * 1024;
const POLL_INTERVAL_MS: u64 = 10;

/// Audio container/encoding label used as provider metadata only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Wav,
    Mp3,
    Flac,
    Ogg,
    #[serde(rename = "pcm16le")]
    Pcm16Le,
}

impl AudioFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Pcm16Le => "pcm16le",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::Mp3 => "audio/mpeg",
            Self::Flac => "audio/flac",
            Self::Ogg => "audio/ogg",
            Self::Pcm16Le => "audio/L16",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Pcm16Le => "pcm",
        }
    }

    /// Parse a declared format label. Only the documented labels are accepted.
    pub fn parse(value: &str) -> Result<Self, VoiceError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "wav" => Ok(Self::Wav),
            "mp3" => Ok(Self::Mp3),
            "flac" => Ok(Self::Flac),
            "ogg" | "ogg_vorbis" | "vorbis" => Ok(Self::Ogg),
            "pcm16le" | "pcm_s16le" | "pcm" => Ok(Self::Pcm16Le),
            _ => Err(VoiceError::UnsupportedFormat(value.to_owned())),
        }
    }
}

/// Typed voice failure shared by all providers and command adapters.
#[derive(Debug, Error)]
pub enum VoiceError {
    #[error("TTS input text is empty")]
    EmptyText,
    #[error("TTS input text is {bytes} bytes, exceeding the {max}-byte limit")]
    TextTooLarge { bytes: usize, max: usize },
    #[error("STT input audio is empty")]
    EmptyAudio,
    #[error("STT input audio is {bytes} bytes, exceeding the {max}-byte limit")]
    AudioTooLarge { bytes: u64, max: u64 },
    #[error("language tag is invalid")]
    InvalidLanguage,
    #[error("voice profile is invalid")]
    InvalidVoiceProfile,
    #[error("TTS rate {value}% is outside the accepted {min}-{max}% range")]
    InvalidRatePercent { value: u16, min: u16, max: u16 },
    #[error("TTS volume {value}% is outside the accepted {min}-{max}% range")]
    InvalidVolumePercent { value: u16, min: u16, max: u16 },
    #[error("audio format '{0}' is not supported")]
    UnsupportedFormat(String),
    #[error("voice command program is empty")]
    EmptyCommand,
    #[error("voice command has {count} arguments, exceeding the {max} limit")]
    TooManyArguments { count: usize, max: usize },
    #[error("voice command argument {index} is {len} bytes, exceeding the {max}-byte limit")]
    ArgumentTooLong {
        index: usize,
        len: usize,
        max: usize,
    },
    #[error("voice command argument {index} contains a NUL byte")]
    InvalidArgument { index: usize },
    #[error("voice command timeout must be between 1 and {max_ms} ms")]
    InvalidTimeout { max_ms: u64 },
    #[error("voice command output limit must be between 1 and {max} bytes")]
    InvalidOutputLimit { max: usize },
    #[error("voice command failed to start: {program}: {message}")]
    Spawn { program: String, message: String },
    #[error("voice command I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("voice command timed out after {timeout_ms} ms")]
    Timeout { timeout_ms: u64 },
    #[error("voice command exited with status {status}{stderr}")]
    NonZeroExit { status: String, stderr: String },
    #[error("voice command produced more than {limit} bytes of output")]
    OutputTooLarge { limit: usize },
    #[error("voice command produced no output")]
    EmptyOutput,
    #[error("SAPI recognition produced an empty transcript")]
    EmptyTranscript,
    #[error("SAPI transcript is {bytes} bytes, exceeding the {max}-byte limit")]
    SapiTranscriptTooLarge { bytes: usize, max: usize },
    #[error("STT transcript is not valid UTF-8")]
    InvalidUtf8,
    #[error("Windows SAPI text-to-speech is unavailable: {reason}")]
    SapiUnavailable { reason: String },
    #[error("speech-to-text is unavailable: {reason}")]
    SttUnavailable { reason: String },
    #[error("synthesized audio is invalid: {reason}")]
    InvalidAudio { reason: &'static str },
    #[error("synthesized audio is {bytes} bytes, exceeding the {max}-byte limit")]
    SapiAudioTooLarge { bytes: usize, max: usize },
}

/// Optional additive speaking style for a TTS request.
///
/// The default is exactly the legacy behavior: 100% rate and 100% volume, which
/// the SAPI provider maps to SAPI rate 0 and volume 100. A style can be supplied
/// directly, as percentages, or derived from a presentation
/// [`VoiceProjection`] without changing that default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsStyle {
    pub rate_percent: u16,
    pub volume_percent: u16,
}

impl Default for TtsStyle {
    fn default() -> Self {
        Self {
            rate_percent: DEFAULT_TTS_RATE_PERCENT,
            volume_percent: DEFAULT_TTS_VOLUME_PERCENT,
        }
    }
}

impl TtsStyle {
    pub const fn new(rate_percent: u16, volume_percent: u16) -> Self {
        Self {
            rate_percent,
            volume_percent,
        }
    }

    /// Derive a style from an existing presentation voice projection.
    pub fn from_projection(projection: &VoiceProjection) -> Self {
        Self {
            rate_percent: projection.rate_percent,
            volume_percent: projection.volume_percent,
        }
    }

    /// Validate both percentages against the accepted bounds.
    pub fn validate(&self) -> Result<(), VoiceError> {
        if !(MIN_TTS_RATE_PERCENT..=MAX_TTS_RATE_PERCENT).contains(&self.rate_percent) {
            return Err(VoiceError::InvalidRatePercent {
                value: self.rate_percent,
                min: MIN_TTS_RATE_PERCENT,
                max: MAX_TTS_RATE_PERCENT,
            });
        }
        if !(MIN_TTS_VOLUME_PERCENT..=MAX_TTS_VOLUME_PERCENT).contains(&self.volume_percent) {
            return Err(VoiceError::InvalidVolumePercent {
                value: self.volume_percent,
                min: MIN_TTS_VOLUME_PERCENT,
                max: MAX_TTS_VOLUME_PERCENT,
            });
        }
        Ok(())
    }
}

/// A TTS request: non-empty text plus a declared output format.
#[derive(Debug, Clone)]
pub struct TtsRequest<'a> {
    pub text: &'a str,
    pub format: AudioFormat,
    pub voice: Option<&'a VoiceProfile>,
    pub language: Option<&'a str>,
    pub style: Option<TtsStyle>,
}

impl<'a> TtsRequest<'a> {
    pub fn new(text: &'a str, format: AudioFormat) -> Self {
        Self {
            text,
            format,
            voice: None,
            language: None,
            style: None,
        }
    }

    pub fn with_voice(mut self, voice: &'a VoiceProfile) -> Self {
        self.voice = Some(voice);
        self
    }

    pub fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    /// Attach an explicit additive style. `None` keeps the legacy default.
    pub fn with_style(mut self, style: TtsStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Attach a style from raw percentages.
    pub fn with_style_percent(mut self, rate_percent: u16, volume_percent: u16) -> Self {
        self.style = Some(TtsStyle::new(rate_percent, volume_percent));
        self
    }

    /// Derive the style from an existing presentation voice projection.
    pub fn with_projection(mut self, projection: &VoiceProjection) -> Self {
        self.style = Some(TtsStyle::from_projection(projection));
        self
    }

    pub fn validate(&self) -> Result<(), VoiceError> {
        validate_tts_text(self.text)?;
        validate_language(self.language)?;
        if let Some(voice) = self.voice {
            validate_voice_profile(voice)?;
        }
        if let Some(style) = self.style {
            style.validate()?;
        }
        Ok(())
    }
}

/// Synthesized audio plus the format metadata declared by the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsResult {
    pub provider: String,
    pub voice_id: Option<String>,
    pub format: AudioFormat,
    pub media_type: String,
    pub audio: Vec<u8>,
}

/// An STT request: audio bytes plus their declared format.
#[derive(Debug, Clone)]
pub struct SttRequest<'a> {
    pub audio: &'a [u8],
    pub format: AudioFormat,
    pub language: Option<&'a str>,
}

impl<'a> SttRequest<'a> {
    pub fn new(audio: &'a [u8], format: AudioFormat) -> Self {
        Self {
            audio,
            format,
            language: None,
        }
    }

    pub fn with_language(mut self, language: &'a str) -> Self {
        self.language = Some(language);
        self
    }

    pub fn validate(&self) -> Result<(), VoiceError> {
        validate_audio_len(self.audio.len() as u64)?;
        validate_language(self.language)?;
        Ok(())
    }
}

/// A transcript plus the format metadata declared by the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttResult {
    pub provider: String,
    pub transcript: String,
    pub format: AudioFormat,
    pub language: Option<String>,
}

/// Provider-neutral text-to-speech contract.
pub trait TtsProvider: std::fmt::Debug + Send + Sync {
    fn name(&self) -> &str;
    fn synthesize(&self, request: &TtsRequest<'_>) -> Result<TtsResult, VoiceError>;
}

/// Provider-neutral speech-to-text contract.
pub trait SttProvider: std::fmt::Debug + Send + Sync {
    fn name(&self) -> &str;
    fn transcribe(&self, request: &SttRequest<'_>) -> Result<SttResult, VoiceError>;
}

/// Validate non-empty TTS text and its byte-size bound.
pub fn validate_tts_text(text: &str) -> Result<(), VoiceError> {
    if text.trim().is_empty() {
        return Err(VoiceError::EmptyText);
    }
    if text.len() > MAX_TTS_TEXT_BYTES {
        return Err(VoiceError::TextTooLarge {
            bytes: text.len(),
            max: MAX_TTS_TEXT_BYTES,
        });
    }
    Ok(())
}

/// Validate non-empty STT audio and its byte-size bound.
pub fn validate_audio_len(bytes: u64) -> Result<(), VoiceError> {
    if bytes == 0 {
        return Err(VoiceError::EmptyAudio);
    }
    if bytes > MAX_STT_AUDIO_BYTES {
        return Err(VoiceError::AudioTooLarge {
            bytes,
            max: MAX_STT_AUDIO_BYTES,
        });
    }
    Ok(())
}

fn validate_language(language: Option<&str>) -> Result<(), VoiceError> {
    if let Some(language) = language {
        let trimmed = language.trim();
        if trimmed.is_empty()
            || trimmed.chars().count() > MAX_VOICE_LANGUAGE_CHARS
            || trimmed.chars().any(char::is_control)
        {
            return Err(VoiceError::InvalidLanguage);
        }
    }
    Ok(())
}

fn validate_voice_profile(voice: &VoiceProfile) -> Result<(), VoiceError> {
    if voice.voice_id.trim().is_empty() || voice.voice_id.chars().count() > MAX_VOICE_ID_CHARS {
        return Err(VoiceError::InvalidVoiceProfile);
    }
    if voice.display_name.trim().is_empty()
        || voice.display_name.chars().count() > MAX_VOICE_ID_CHARS
    {
        return Err(VoiceError::InvalidVoiceProfile);
    }
    if let Some(sample) = &voice.sample_asset_id {
        if sample.trim().is_empty() || sample.chars().count() > MAX_VOICE_ID_CHARS {
            return Err(VoiceError::InvalidVoiceProfile);
        }
    }
    Ok(())
}

/// Owner-configured local command, with fixed arguments and execution bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    program: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    max_output_bytes: usize,
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            timeout: Duration::from_millis(DEFAULT_COMMAND_TIMEOUT_MS),
            max_output_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
        }
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    /// Validate program, argument vector, timeout, and output cap bounds.
    pub fn validate(&self) -> Result<(), VoiceError> {
        if self.program.as_os_str().is_empty() {
            return Err(VoiceError::EmptyCommand);
        }
        if self.args.len() > MAX_COMMAND_ARGS {
            return Err(VoiceError::TooManyArguments {
                count: self.args.len(),
                max: MAX_COMMAND_ARGS,
            });
        }
        for (index, arg) in self.args.iter().enumerate() {
            if arg.contains('\0') {
                return Err(VoiceError::InvalidArgument { index });
            }
            if arg.len() > MAX_COMMAND_ARG_CHARS {
                return Err(VoiceError::ArgumentTooLong {
                    index,
                    len: arg.len(),
                    max: MAX_COMMAND_ARG_CHARS,
                });
            }
        }
        let max_timeout = Duration::from_millis(MAX_COMMAND_TIMEOUT_MS);
        if self.timeout.is_zero() || self.timeout > max_timeout {
            return Err(VoiceError::InvalidTimeout {
                max_ms: MAX_COMMAND_TIMEOUT_MS,
            });
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_COMMAND_OUTPUT_BYTES {
            return Err(VoiceError::InvalidOutputLimit {
                max: MAX_COMMAND_OUTPUT_BYTES,
            });
        }
        Ok(())
    }
}

/// TTS adapter that pipes text to an owner-configured executable's stdin and
/// reads audio bytes from its stdout.
#[derive(Debug, Clone)]
pub struct CommandTtsProvider {
    name: String,
    spec: CommandSpec,
}

impl CommandTtsProvider {
    pub fn new(name: impl Into<String>, spec: CommandSpec) -> Self {
        Self {
            name: name.into(),
            spec,
        }
    }

    pub fn spec(&self) -> &CommandSpec {
        &self.spec
    }
}

impl TtsProvider for CommandTtsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn synthesize(&self, request: &TtsRequest<'_>) -> Result<TtsResult, VoiceError> {
        request.validate()?;
        let audio = run_command(&self.spec, request.text.as_bytes())?;
        if audio.is_empty() {
            return Err(VoiceError::EmptyOutput);
        }
        Ok(TtsResult {
            provider: self.name.clone(),
            voice_id: request.voice.map(|voice| voice.voice_id.clone()),
            format: request.format,
            media_type: request.format.media_type().to_owned(),
            audio,
        })
    }
}

/// STT adapter that pipes audio bytes to an owner-configured executable's
/// stdin and reads a UTF-8 transcript from its stdout.
#[derive(Debug, Clone)]
pub struct CommandSttProvider {
    name: String,
    spec: CommandSpec,
}

impl CommandSttProvider {
    pub fn new(name: impl Into<String>, spec: CommandSpec) -> Self {
        Self {
            name: name.into(),
            spec,
        }
    }

    pub fn spec(&self) -> &CommandSpec {
        &self.spec
    }
}

impl SttProvider for CommandSttProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn transcribe(&self, request: &SttRequest<'_>) -> Result<SttResult, VoiceError> {
        request.validate()?;
        let output = run_command(&self.spec, request.audio)?;
        let transcript = String::from_utf8(output).map_err(|_| VoiceError::InvalidUtf8)?;
        Ok(SttResult {
            provider: self.name.clone(),
            transcript,
            format: request.format,
            language: request.language.map(str::to_owned),
        })
    }
}

fn run_command(spec: &CommandSpec, input: &[u8]) -> Result<Vec<u8>, VoiceError> {
    spec.validate()?;
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| VoiceError::Spawn {
            program: spec.program.display().to_string(),
            message: error.to_string(),
        })?;

    let mut stdin = child.stdin.take().ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "voice command stdin was not captured",
        ))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "voice command stdout was not captured",
        ))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "voice command stderr was not captured",
        ))
    })?;

    let deadline = Instant::now() + spec.timeout;
    let max_output = spec.max_output_bytes;

    let (status, timed_out, wait_error, stdout, stderr) = thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let _ = stdin.write_all(input);
            let _ = stdin.flush();
        });
        let stdout_reader = scope.spawn(move || read_bounded(stdout, max_output));
        let stderr_reader = scope.spawn(move || read_bounded(stderr, MAX_STDERR_BYTES));

        let mut timed_out = false;
        let mut wait_error = None;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(error) => {
                    wait_error = Some(error);
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        };

        let _ = writer.join();
        let stdout = join_reader(stdout_reader);
        let stderr = join_reader(stderr_reader);
        (status, timed_out, wait_error, stdout, stderr)
    });

    if let Some(error) = wait_error {
        return Err(VoiceError::Io(error));
    }
    if timed_out {
        return Err(VoiceError::Timeout {
            timeout_ms: spec.timeout.as_millis().min(u64::MAX as u128) as u64,
        });
    }
    let (stdout, output_exceeded) = stdout?;
    let (stderr, _) = stderr?;
    let status = status.ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "voice command status was not captured",
        ))
    })?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        let suffix = if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        };
        return Err(VoiceError::NonZeroExit {
            status: status.to_string(),
            stderr: suffix,
        });
    }
    if output_exceeded {
        return Err(VoiceError::OutputTooLarge {
            limit: spec.max_output_bytes,
        });
    }
    Ok(stdout)
}

fn join_reader<'scope>(
    handle: thread::ScopedJoinHandle<'scope, std::io::Result<(Vec<u8>, bool)>>,
) -> Result<(Vec<u8>, bool), VoiceError> {
    match handle.join() {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(VoiceError::Io(error)),
        Err(_) => Err(VoiceError::Io(std::io::Error::other(
            "voice command reader thread panicked",
        ))),
    }
}

fn read_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut exceeded = false;
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        if buffer.len().saturating_add(read) > limit {
            exceeded = true;
            continue;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    Ok((buffer, exceeded))
}

/// Result of probing one voice capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityStatus {
    /// True only when live audio was actually verified. Configuration alone is
    /// never treated as working audio.
    pub available: bool,
    /// True only when a live, end-to-end audio check succeeded.
    pub verified: bool,
    /// Path of a recognized executable found on `PATH`, if any.
    pub detected_executable: Option<String>,
    /// Human-readable explanation of the probe result.
    pub reason: String,
}

/// Conservative availability report for TTS and STT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceAvailability {
    pub schema: String,
    pub tts: CapabilityStatus,
    pub stt: CapabilityStatus,
}

impl VoiceAvailability {
    /// True only when both capabilities have been live-verified.
    pub fn is_available(&self) -> bool {
        self.tts.available && self.stt.available
    }
}

/// Probe for recognized local TTS/STT executables without performing audio.
///
/// The probe intentionally reports `available = false` even when an executable
/// is present, because detecting configuration is not the same as verifying
/// live audio. In this environment no TTS/STT engine is installed, so both
/// capabilities report unavailable with an explanatory reason.
pub fn probe_availability() -> VoiceAvailability {
    VoiceAvailability {
        schema: VOICE_AVAILABILITY_SCHEMA.to_owned(),
        tts: probe_capability(
            "TTS",
            &["piper", "piper-tts", "qwen3-tts"],
            "Ollama provides LLM inference only and ships no TTS model",
        ),
        stt: probe_capability(
            "STT",
            &[
                "whisper",
                "faster-whisper",
                "whisper-cli",
                "whisper-ctranslate2",
            ],
            "Ollama provides LLM inference only and ships no STT model",
        ),
    }
}

fn probe_capability(label: &str, names: &[&str], note: &str) -> CapabilityStatus {
    for name in names {
        if let Some(path) = find_executable(name) {
            return CapabilityStatus {
                available: false,
                verified: false,
                detected_executable: Some(path.display().to_string()),
                reason: format!(
                    "{label} executable '{name}' detected at {} but live audio was not \
                     verified; configuration alone is not treated as working audio. {note}",
                    path.display()
                ),
            };
        }
    }
    CapabilityStatus {
        available: false,
        verified: false,
        detected_executable: None,
        reason: format!(
            "no local {label} executable found (looked for {}); {note}; live audio \
             unavailable in this environment",
            names.join(", ")
        ),
    }
}

/// Find an executable by name on `PATH` without spawning anything.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat", "com"] {
            let candidate = directory.join(format!("{name}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// A Core-bound view of its configured voice profile and sample asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceBinding {
    pub core_uuid: String,
    pub voice_id: String,
    pub display_name: String,
    pub sample_asset_id: Option<String>,
    pub sample_sha256: Option<String>,
    pub sample_bytes: Option<u64>,
}

/// Windows SAPI text-to-speech provider. Uses the built-in
/// `System.Speech` assembly through a fixed PowerShell script; no model is
/// downloaded and no audio is played.
pub const SAPI_TTS_PROVIDER_NAME: &str = "windows-sapi-tts";
/// Windows SAPI file-based speech-to-text provider. Recognizes a WAV file with
/// the built-in `System.Speech.Recognition` engine; no microphone is captured.
pub const SAPI_STT_PROVIDER_NAME: &str = "windows-sapi-stt";
/// Default timeout for a single SAPI synthesis run, in milliseconds.
pub const DEFAULT_SAPI_TIMEOUT_MS: u64 = 30_000;
/// Maximum accepted size of a synthesized WAV, in bytes (64 MiB).
pub const MAX_SAPI_AUDIO_BYTES: usize = 64 * 1024 * 1024;
/// Maximum accepted size of a SAPI transcript file, in bytes (1 MiB).
pub const MAX_SAPI_TRANSCRIPT_BYTES: usize = 1024 * 1024;

const SAPI_STDOUT_LIMIT_BYTES: usize = 8 * 1024;

/// Text-to-speech provider backed by the Windows `System.Speech` synthesizer.
///
/// The input text is written to a UTF-8 file and the output path is passed as
/// an argument to a fixed PowerShell script. The script path, text path, and
/// output path are delivered as an argument vector (never a shell string), and
/// the resulting WAV file is read back and validated. No audio is played.
#[derive(Debug, Clone)]
pub struct SapiTtsProvider {
    name: String,
    timeout: Duration,
    max_audio_bytes: usize,
    style: Option<TtsStyle>,
}

impl SapiTtsProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            timeout: Duration::from_millis(DEFAULT_SAPI_TIMEOUT_MS),
            max_audio_bytes: MAX_SAPI_AUDIO_BYTES,
            style: None,
        }
    }

    /// Provider using the default `windows-sapi-tts` name.
    pub fn system_default() -> Self {
        Self::new(SAPI_TTS_PROVIDER_NAME)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_audio_bytes(mut self, max_audio_bytes: usize) -> Self {
        self.max_audio_bytes = max_audio_bytes;
        self
    }

    /// Set a default additive style for every synthesis request.
    ///
    /// A style on an individual [`TtsRequest`] takes precedence. Leaving this
    /// unset preserves the legacy 100% rate / 100% volume default.
    pub fn with_style(mut self, style: TtsStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Set a default style from raw percentages.
    pub fn with_style_percent(mut self, rate_percent: u16, volume_percent: u16) -> Self {
        self.style = Some(TtsStyle::new(rate_percent, volume_percent));
        self
    }

    /// Derive a default style from an existing presentation voice projection.
    pub fn with_projection(mut self, projection: &VoiceProjection) -> Self {
        self.style = Some(TtsStyle::from_projection(projection));
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn max_audio_bytes(&self) -> usize {
        self.max_audio_bytes
    }

    pub fn style(&self) -> Option<TtsStyle> {
        self.style
    }

    fn validate_limits(&self) -> Result<(), VoiceError> {
        if self.timeout.is_zero() || self.timeout > Duration::from_millis(MAX_COMMAND_TIMEOUT_MS) {
            return Err(VoiceError::InvalidTimeout {
                max_ms: MAX_COMMAND_TIMEOUT_MS,
            });
        }
        if self.max_audio_bytes == 0 || self.max_audio_bytes > MAX_SAPI_AUDIO_BYTES {
            return Err(VoiceError::InvalidOutputLimit {
                max: MAX_SAPI_AUDIO_BYTES,
            });
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn synthesize_wav(&self, _text: &str, _style: Option<TtsStyle>) -> Result<Vec<u8>, VoiceError> {
        Err(VoiceError::SapiUnavailable {
            reason: "Windows System.Speech is only available on Windows".to_owned(),
        })
    }

    #[cfg(windows)]
    fn synthesize_wav(&self, text: &str, style: Option<TtsStyle>) -> Result<Vec<u8>, VoiceError> {
        use std::fs;

        let workspace = SapiWorkspace::create()?;
        let script_path = workspace.dir.join("sapi_synthesize.ps1");
        let text_path = workspace.dir.join("input.txt");
        let out_path = workspace.dir.join("output.wav");
        fs::write(&script_path, SAPI_SCRIPT)?;
        fs::write(&text_path, text.as_bytes())?;

        let args = sapi_tts_args(&script_path, &text_path, &out_path, style);

        let outcome = run_sapi_process(&powershell_program(), &args, self.timeout)?;
        if !outcome.status.success() {
            let stderr = String::from_utf8_lossy(&outcome.stderr).trim().to_owned();
            let lower = stderr.to_ascii_lowercase();
            let unavailable = stderr.is_empty()
                || lower.contains("no voice")
                || lower.contains("not installed")
                || lower.contains("cannot find")
                || lower.contains("assembly")
                || lower.contains("system.speech")
                || lower.contains("exception");
            if unavailable {
                return Err(VoiceError::SapiUnavailable {
                    reason: if stderr.is_empty() {
                        format!("powershell exited with {}", outcome.status)
                    } else {
                        stderr
                    },
                });
            }
            let suffix = format!(": {stderr}");
            return Err(VoiceError::NonZeroExit {
                status: outcome.status.to_string(),
                stderr: suffix,
            });
        }

        let bytes = fs::read(&out_path)?;
        if bytes.is_empty() {
            return Err(VoiceError::EmptyOutput);
        }
        if bytes.len() > self.max_audio_bytes {
            return Err(VoiceError::SapiAudioTooLarge {
                bytes: bytes.len(),
                max: self.max_audio_bytes,
            });
        }
        validate_wav_bytes(&bytes)?;
        Ok(bytes)
    }
}

impl TtsProvider for SapiTtsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn synthesize(&self, request: &TtsRequest<'_>) -> Result<TtsResult, VoiceError> {
        request.validate()?;
        self.validate_limits()?;
        if request.format != AudioFormat::Wav {
            return Err(VoiceError::UnsupportedFormat(
                request.format.as_str().to_owned(),
            ));
        }
        let style = request.style.or(self.style);
        if let Some(style) = style {
            style.validate()?;
        }
        let audio = self.synthesize_wav(request.text, style)?;
        Ok(TtsResult {
            provider: self.name.clone(),
            voice_id: request.voice.map(|voice| voice.voice_id.clone()),
            format: AudioFormat::Wav,
            media_type: AudioFormat::Wav.media_type().to_owned(),
            audio,
        })
    }
}

/// Speech-to-text provider backed by the Windows `System.Speech.Recognition`
/// engine, recognizing a caller-supplied WAV file.
///
/// This is **file-based recognition**, not microphone capture: the audio bytes
/// are written to a temp WAV file, a fixed PowerShell script calls
/// `SpeechRecognitionEngine.SetInputToWaveFile` and `Recognize()`, and the
/// transcript is read back from an output file. The script and paths are passed
/// as an argument vector (never a shell string) with a timeout and bounded
/// stdout/stderr. No microphone is opened and no audio is played.
#[derive(Debug, Clone)]
pub struct SapiSttProvider {
    name: String,
    timeout: Duration,
    max_transcript_bytes: usize,
}

impl SapiSttProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            timeout: Duration::from_millis(DEFAULT_SAPI_TIMEOUT_MS),
            max_transcript_bytes: MAX_SAPI_TRANSCRIPT_BYTES,
        }
    }

    /// Provider using the default `windows-sapi-stt` name.
    pub fn system_default() -> Self {
        Self::new(SAPI_STT_PROVIDER_NAME)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_transcript_bytes(mut self, max_transcript_bytes: usize) -> Self {
        self.max_transcript_bytes = max_transcript_bytes;
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn max_transcript_bytes(&self) -> usize {
        self.max_transcript_bytes
    }

    fn validate_limits(&self) -> Result<(), VoiceError> {
        if self.timeout.is_zero() || self.timeout > Duration::from_millis(MAX_COMMAND_TIMEOUT_MS) {
            return Err(VoiceError::InvalidTimeout {
                max_ms: MAX_COMMAND_TIMEOUT_MS,
            });
        }
        if self.max_transcript_bytes == 0 || self.max_transcript_bytes > MAX_SAPI_TRANSCRIPT_BYTES {
            return Err(VoiceError::InvalidOutputLimit {
                max: MAX_SAPI_TRANSCRIPT_BYTES,
            });
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn transcribe_wav(&self, _audio: &[u8]) -> Result<String, VoiceError> {
        Err(VoiceError::SttUnavailable {
            reason: "Windows System.Speech recognition is only available on Windows".to_owned(),
        })
    }

    #[cfg(windows)]
    fn transcribe_wav(&self, audio: &[u8]) -> Result<String, VoiceError> {
        use std::ffi::OsString;
        use std::fs;

        let workspace = SapiWorkspace::create()?;
        let script_path = workspace.dir.join("sapi_recognize.ps1");
        let wave_path = workspace.dir.join("input.wav");
        let out_path = workspace.dir.join("transcript.txt");
        fs::write(&script_path, SAPI_STT_SCRIPT)?;
        fs::write(&wave_path, audio)?;

        let args: Vec<OsString> = vec![
            OsString::from("-NoProfile"),
            OsString::from("-ExecutionPolicy"),
            OsString::from("Bypass"),
            OsString::from("-File"),
            script_path.into_os_string(),
            OsString::from("-WaveFile"),
            wave_path.into_os_string(),
            OsString::from("-OutFile"),
            out_path.clone().into_os_string(),
        ];

        let outcome = run_sapi_stt_process(&powershell_program(), &args, self.timeout)?;
        if !outcome.status.success() {
            let stderr = String::from_utf8_lossy(&outcome.stderr).trim().to_owned();
            let lower = stderr.to_ascii_lowercase();
            let unavailable = stderr.is_empty()
                || lower.contains("recognizer")
                || lower.contains("no installed")
                || lower.contains("not installed")
                || lower.contains("cannot find")
                || lower.contains("assembly")
                || lower.contains("system.speech")
                || lower.contains("platform not supported")
                || lower.contains("exception");
            if unavailable {
                return Err(VoiceError::SttUnavailable {
                    reason: if stderr.is_empty() {
                        format!("powershell exited with {}", outcome.status)
                    } else {
                        stderr
                    },
                });
            }
            let suffix = format!(": {stderr}");
            return Err(VoiceError::NonZeroExit {
                status: outcome.status.to_string(),
                stderr: suffix,
            });
        }

        let bytes = match fs::read(&out_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(VoiceError::EmptyTranscript)
            }
            Err(error) => return Err(VoiceError::Io(error)),
        };
        if bytes.len() > self.max_transcript_bytes {
            return Err(VoiceError::SapiTranscriptTooLarge {
                bytes: bytes.len(),
                max: self.max_transcript_bytes,
            });
        }
        let transcript = String::from_utf8(bytes).map_err(|_| VoiceError::InvalidUtf8)?;
        let transcript = transcript.trim().to_owned();
        if transcript.is_empty() {
            return Err(VoiceError::EmptyTranscript);
        }
        Ok(transcript)
    }
}

impl SttProvider for SapiSttProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn transcribe(&self, request: &SttRequest<'_>) -> Result<SttResult, VoiceError> {
        request.validate()?;
        self.validate_limits()?;
        if request.format != AudioFormat::Wav {
            return Err(VoiceError::UnsupportedFormat(
                request.format.as_str().to_owned(),
            ));
        }
        validate_wav_bytes(request.audio)?;
        let transcript = self.transcribe_wav(request.audio)?;
        Ok(SttResult {
            provider: self.name.clone(),
            transcript,
            format: AudioFormat::Wav,
            language: request.language.map(str::to_owned),
        })
    }
}

/// Map a validated TTS rate percentage to the SAPI range (-10 ..= 10).
///
/// 100% maps to 0, so the default is byte-for-byte the legacy behavior.
fn sapi_rate_from_percent(rate_percent: u16) -> i32 {
    let delta = i32::from(rate_percent) - i32::from(DEFAULT_TTS_RATE_PERCENT);
    (delta / 10).clamp(-10, 10)
}

#[cfg(windows)]
fn sapi_tts_args(
    script_path: &Path,
    text_path: &Path,
    out_path: &Path,
    style: Option<TtsStyle>,
) -> Vec<std::ffi::OsString> {
    use std::ffi::OsString;

    let style = style.unwrap_or_default();
    let rate = sapi_rate_from_percent(style.rate_percent);
    vec![
        OsString::from("-NoProfile"),
        OsString::from("-ExecutionPolicy"),
        OsString::from("Bypass"),
        OsString::from("-File"),
        script_path.as_os_str().to_os_string(),
        OsString::from("-TextFile"),
        text_path.as_os_str().to_os_string(),
        OsString::from("-OutFile"),
        out_path.as_os_str().to_os_string(),
        OsString::from("-Rate"),
        OsString::from(rate.to_string()),
        OsString::from("-Volume"),
        OsString::from(style.volume_percent.to_string()),
    ]
}

#[cfg(windows)]
const SAPI_SCRIPT: &str = r#"param(
    [Parameter(Mandatory = $true)][string]$TextFile,
    [Parameter(Mandatory = $true)][string]$OutFile,
    [int]$Rate = 0,
    [int]$Volume = 100
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$text = [System.IO.File]::ReadAllText($TextFile, [System.Text.Encoding]::UTF8)
if ([string]::IsNullOrWhiteSpace($text)) {
    throw 'SAPI input text is empty'
}
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(
    22050,
    [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
    [System.Speech.AudioFormat.AudioChannel]::Mono
)
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
try {
    $synth.Rate = [Math]::Max(-10, [Math]::Min(10, $Rate))
    $synth.Volume = [Math]::Max(0, [Math]::Min(100, $Volume))
    $synth.SetOutputToWaveFile($OutFile, $format)
    $synth.Speak($text)
} finally {
    $synth.Dispose()
}
"#;

#[cfg(windows)]
const SAPI_STT_SCRIPT: &str = r#"param(
    [Parameter(Mandatory = $true)][string]$WaveFile,
    [Parameter(Mandatory = $true)][string]$OutFile
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$recognizers = [System.Speech.Recognition.SpeechRecognitionEngine]::InstalledRecognizers()
if ($null -eq $recognizers -or $recognizers.Count -eq 0) {
    throw 'No installed SAPI speech recognizer is available'
}
$engine = New-Object System.Speech.Recognition.SpeechRecognitionEngine($recognizers[0])
try {
    $engine.LoadGrammar((New-Object System.Speech.Recognition.DictationGrammar))
    $engine.SetInputToWaveFile($WaveFile)
    $result = $engine.Recognize()
    $text = ''
    if ($null -ne $result) {
        $text = $result.Text
    }
    [System.IO.File]::WriteAllText($OutFile, $text, (New-Object System.Text.UTF8Encoding($false)))
} finally {
    $engine.Dispose()
}
"#;

#[cfg(windows)]
struct SapiWorkspace {
    dir: PathBuf,
}

#[cfg(windows)]
impl SapiWorkspace {
    fn create() -> Result<Self, VoiceError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "aegis-sapi-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }
}

#[cfg(windows)]
impl Drop for SapiWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(windows)]
fn powershell_program() -> PathBuf {
    if let Some(root) = std::env::var_os("SystemRoot") {
        let candidate = PathBuf::from(root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if candidate.is_file() {
            return candidate;
        }
    }
    find_executable("powershell").unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

#[cfg(windows)]
struct SapiProcessOutcome {
    status: std::process::ExitStatus,
    stderr: Vec<u8>,
}

#[cfg(windows)]
fn run_sapi_process(
    program: &Path,
    args: &[std::ffi::OsString],
    timeout: Duration,
) -> Result<SapiProcessOutcome, VoiceError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| VoiceError::SapiUnavailable {
            reason: format!("failed to start {}: {error}", program.display()),
        })?;
    wait_bounded_process(child, timeout)
}

#[cfg(windows)]
fn run_sapi_stt_process(
    program: &Path,
    args: &[std::ffi::OsString],
    timeout: Duration,
) -> Result<SapiProcessOutcome, VoiceError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| VoiceError::SttUnavailable {
            reason: format!("failed to start {}: {error}", program.display()),
        })?;
    wait_bounded_process(child, timeout)
}

#[cfg(windows)]
fn wait_bounded_process(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<SapiProcessOutcome, VoiceError> {
    let stdout = child.stdout.take().ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "SAPI process stdout was not captured",
        ))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "SAPI process stderr was not captured",
        ))
    })?;

    let deadline = Instant::now() + timeout;
    let (status, timed_out, wait_error, stdout, stderr) = thread::scope(|scope| {
        let stdout_reader = scope.spawn(move || read_bounded(stdout, SAPI_STDOUT_LIMIT_BYTES));
        let stderr_reader = scope.spawn(move || read_bounded(stderr, MAX_STDERR_BYTES));

        let mut timed_out = false;
        let mut wait_error = None;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(error) => {
                    wait_error = Some(error);
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        };

        let stdout = join_reader(stdout_reader);
        let stderr = join_reader(stderr_reader);
        (status, timed_out, wait_error, stdout, stderr)
    });

    if let Some(error) = wait_error {
        return Err(VoiceError::Io(error));
    }
    if timed_out {
        return Err(VoiceError::Timeout {
            timeout_ms: timeout.as_millis().min(u64::MAX as u128) as u64,
        });
    }
    let (_, stdout_exceeded) = stdout?;
    let (stderr, _) = stderr?;
    let status = status.ok_or_else(|| {
        VoiceError::Io(std::io::Error::other(
            "SAPI process status was not captured",
        ))
    })?;
    if stdout_exceeded {
        return Err(VoiceError::OutputTooLarge {
            limit: SAPI_STDOUT_LIMIT_BYTES,
        });
    }
    Ok(SapiProcessOutcome { status, stderr })
}

/// Validate that `bytes` is a plausible non-empty RIFF/WAVE file with an audio
/// `data` chunk inside the declared size bounds.
fn validate_wav_bytes(bytes: &[u8]) -> Result<(), VoiceError> {
    if bytes.len() < 44 {
        return Err(VoiceError::InvalidAudio {
            reason: "WAV is shorter than a 44-byte header",
        });
    }
    if &bytes[0..4] != b"RIFF" {
        return Err(VoiceError::InvalidAudio {
            reason: "missing RIFF signature",
        });
    }
    if &bytes[8..12] != b"WAVE" {
        return Err(VoiceError::InvalidAudio {
            reason: "missing WAVE signature",
        });
    }
    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let declared = match riff_size.checked_add(8) {
        Some(declared) => declared,
        None => {
            return Err(VoiceError::InvalidAudio {
                reason: "RIFF chunk size overflows",
            })
        }
    };
    if declared > bytes.len() {
        return Err(VoiceError::InvalidAudio {
            reason: "RIFF chunk size exceeds file length",
        });
    }

    let mut offset = 12usize;
    let mut has_fmt = false;
    let mut data_len = 0usize;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        let end = match body.checked_add(size) {
            Some(end) if end <= bytes.len() => end,
            _ => {
                return Err(VoiceError::InvalidAudio {
                    reason: "WAV chunk size exceeds file length",
                })
            }
        };
        if id == b"fmt " {
            if size < 16 {
                return Err(VoiceError::InvalidAudio {
                    reason: "WAV fmt chunk is too small",
                });
            }
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let sample_rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            if channels == 0 || sample_rate == 0 {
                return Err(VoiceError::InvalidAudio {
                    reason: "WAV fmt chunk has invalid channel or sample rate",
                });
            }
            has_fmt = true;
        } else if id == b"data" {
            data_len = size;
        }
        offset = end + (size % 2);
    }

    if !has_fmt {
        return Err(VoiceError::InvalidAudio {
            reason: "WAV has no fmt chunk",
        });
    }
    if data_len == 0 {
        return Err(VoiceError::InvalidAudio {
            reason: "WAV data chunk is empty",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pcm_wav() -> Vec<u8> {
        let data = [0u8];
        let total = 12 + (8 + 16) + (8 + data.len());
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&((total - 8) as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&22050u32.to_le_bytes());
        wav.extend_from_slice(&22050u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    #[test]
    fn wav_validation_accepts_minimal_pcm_and_rejects_bad_headers() {
        let wav = minimal_pcm_wav();
        assert!(validate_wav_bytes(&wav).is_ok());

        assert!(matches!(
            validate_wav_bytes(b"not a wav"),
            Err(VoiceError::InvalidAudio { .. })
        ));

        let mut bad_riff = wav.clone();
        bad_riff[0..4].copy_from_slice(b"NOPE");
        assert!(matches!(
            validate_wav_bytes(&bad_riff),
            Err(VoiceError::InvalidAudio { .. })
        ));

        let mut bad_wave = wav.clone();
        bad_wave[8..12].copy_from_slice(b"XXXX");
        assert!(matches!(
            validate_wav_bytes(&bad_wave),
            Err(VoiceError::InvalidAudio { .. })
        ));

        let mut truncated = wav.clone();
        truncated.truncate(20);
        assert!(matches!(
            validate_wav_bytes(&truncated),
            Err(VoiceError::InvalidAudio { .. })
        ));

        let mut oversized = wav.clone();
        oversized[4..8].copy_from_slice(&(u32::MAX).to_le_bytes());
        assert!(matches!(
            validate_wav_bytes(&oversized),
            Err(VoiceError::InvalidAudio { .. })
        ));
    }

    #[test]
    fn tts_style_defaults_and_bounds() {
        assert_eq!(TtsStyle::default(), TtsStyle::new(100, 100));
        assert!(TtsStyle::new(MIN_TTS_RATE_PERCENT, MIN_TTS_VOLUME_PERCENT)
            .validate()
            .is_ok());
        assert!(TtsStyle::new(MAX_TTS_RATE_PERCENT, MAX_TTS_VOLUME_PERCENT)
            .validate()
            .is_ok());
        assert!(matches!(
            TtsStyle::new(MIN_TTS_RATE_PERCENT - 1, 100).validate(),
            Err(VoiceError::InvalidRatePercent { .. })
        ));
        assert!(matches!(
            TtsStyle::new(MAX_TTS_RATE_PERCENT + 1, 100).validate(),
            Err(VoiceError::InvalidRatePercent { .. })
        ));
        assert!(matches!(
            TtsStyle::new(100, MAX_TTS_VOLUME_PERCENT + 1).validate(),
            Err(VoiceError::InvalidVolumePercent { .. })
        ));
    }

    #[test]
    fn sapi_rate_maps_default_to_zero_and_bounded_percentages() {
        assert_eq!(sapi_rate_from_percent(100), 0);
        assert_eq!(sapi_rate_from_percent(150), 5);
        assert_eq!(sapi_rate_from_percent(60), -4);
        assert_eq!(sapi_rate_from_percent(MIN_TTS_RATE_PERCENT), -5);
        assert_eq!(sapi_rate_from_percent(MAX_TTS_RATE_PERCENT), 10);
    }

    #[cfg(windows)]
    #[test]
    fn sapi_tts_args_carry_style_without_shell_interpolation() {
        let script = Path::new("C:\\temp dir\\sapi.ps1");
        let text = Path::new("C:\\temp dir\\input.txt");
        let out = Path::new("C:\\temp dir\\output.wav");

        let args = sapi_tts_args(script, text, out, None);
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let rate = rendered
            .iter()
            .position(|arg| arg == "-Rate")
            .expect("rate");
        assert_eq!(rendered[rate + 1], "0");
        let volume = rendered
            .iter()
            .position(|arg| arg == "-Volume")
            .expect("volume");
        assert_eq!(rendered[volume + 1], "100");
        assert!(rendered.contains(&"C:\\temp dir\\sapi.ps1".to_owned()));
        assert!(!rendered.iter().any(|arg| arg.contains("-Command")));

        let args = sapi_tts_args(script, text, out, Some(TtsStyle::new(150, 80)));
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let rate = rendered
            .iter()
            .position(|arg| arg == "-Rate")
            .expect("rate");
        assert_eq!(rendered[rate + 1], "5");
        let volume = rendered
            .iter()
            .position(|arg| arg == "-Volume")
            .expect("volume");
        assert_eq!(rendered[volume + 1], "80");
    }

    #[test]
    fn sapi_tts_validates_request_format_and_limits() {
        let provider = SapiTtsProvider::system_default();
        assert_eq!(provider.name(), SAPI_TTS_PROVIDER_NAME);
        assert!(matches!(
            provider.synthesize(&TtsRequest::new("   \t", AudioFormat::Wav)),
            Err(VoiceError::EmptyText)
        ));
        assert!(matches!(
            provider.synthesize(&TtsRequest::new("hello", AudioFormat::Mp3)),
            Err(VoiceError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            SapiTtsProvider::system_default()
                .with_timeout(Duration::from_millis(0))
                .synthesize(&TtsRequest::new("hello", AudioFormat::Wav)),
            Err(VoiceError::InvalidTimeout { .. })
        ));
        assert!(matches!(
            SapiTtsProvider::system_default()
                .with_max_audio_bytes(0)
                .synthesize(&TtsRequest::new("hello", AudioFormat::Wav)),
            Err(VoiceError::InvalidOutputLimit { .. })
        ));
    }

    #[test]
    fn sapi_stt_validates_request_format_and_limits() {
        let provider = SapiSttProvider::system_default();
        assert_eq!(provider.name(), SAPI_STT_PROVIDER_NAME);
        assert!(matches!(
            provider.transcribe(&SttRequest::new(b"", AudioFormat::Wav)),
            Err(VoiceError::EmptyAudio)
        ));
        assert!(matches!(
            provider.transcribe(&SttRequest::new(b"not-really-audio", AudioFormat::Wav)),
            Err(VoiceError::InvalidAudio { .. })
        ));
        assert!(matches!(
            provider.transcribe(&SttRequest::new(b"not-really-audio", AudioFormat::Pcm16Le)),
            Err(VoiceError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            SapiSttProvider::system_default()
                .with_timeout(Duration::from_millis(0))
                .transcribe(&SttRequest::new(b"audio", AudioFormat::Wav)),
            Err(VoiceError::InvalidTimeout { .. })
        ));
        assert!(matches!(
            SapiSttProvider::system_default()
                .with_max_transcript_bytes(0)
                .transcribe(&SttRequest::new(b"audio", AudioFormat::Wav)),
            Err(VoiceError::InvalidOutputLimit { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn live_sapi_tts_writes_valid_wav_when_enabled() {
        if std::env::var("AEGIS_VOICE_LIVE").ok().as_deref() != Some("1") {
            eprintln!("skipping live SAPI TTS: set AEGIS_VOICE_LIVE=1 to enable");
            return;
        }
        let provider = SapiTtsProvider::system_default().with_timeout(Duration::from_secs(30));
        let request = TtsRequest::new("Aegis voice check.", AudioFormat::Wav);
        match provider.synthesize(&request) {
            Ok(result) => {
                assert!(
                    !result.audio.is_empty(),
                    "live SAPI TTS returned empty audio"
                );
                assert_eq!(&result.audio[0..4], b"RIFF");
                assert_eq!(&result.audio[8..12], b"WAVE");
                assert!(result.audio.len() > 44);
                eprintln!(
                    "live SAPI TTS produced {} bytes of WAV audio",
                    result.audio.len()
                );
            }
            Err(VoiceError::SapiUnavailable { reason }) => {
                eprintln!("skipping live SAPI TTS: SAPI unavailable: {reason}");
            }
            Err(other) => panic!("live SAPI TTS failed: {other}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn live_sapi_stt_round_trip_transcribes_synthesized_wav_when_enabled() {
        if std::env::var("AEGIS_VOICE_LIVE").ok().as_deref() != Some("1") {
            eprintln!("skipping live SAPI STT round-trip: set AEGIS_VOICE_LIVE=1 to enable");
            return;
        }
        const PHRASE: &str = "the quick brown fox jumps over the lazy dog";

        let tts = SapiTtsProvider::system_default().with_timeout(Duration::from_secs(30));
        let synthesized = match tts.synthesize(&TtsRequest::new(PHRASE, AudioFormat::Wav)) {
            Ok(result) => result,
            Err(VoiceError::SapiUnavailable { reason }) => {
                eprintln!("skipping live SAPI STT round-trip: TTS unavailable: {reason}");
                return;
            }
            Err(other) => panic!("live SAPI STT round-trip TTS failed: {other}"),
        };
        assert!(!synthesized.audio.is_empty(), "TTS produced empty audio");

        let stt = SapiSttProvider::system_default().with_timeout(Duration::from_secs(30));
        let transcribed = match stt
            .transcribe(&SttRequest::new(&synthesized.audio, AudioFormat::Wav))
        {
            Ok(result) => result,
            Err(VoiceError::SttUnavailable { reason }) => {
                eprintln!("skipping live SAPI STT round-trip: recognizer unavailable: {reason}");
                return;
            }
            Err(other) => panic!("live SAPI STT failed: {other}"),
        };

        let transcript = transcribed.transcript;
        assert!(
            !transcript.trim().is_empty(),
            "live SAPI STT returned an empty transcript"
        );
        eprintln!(
            "live SAPI TTS->STT WAV bytes: {}, transcript: {transcript}",
            synthesized.audio.len()
        );
        let lower = transcript.to_ascii_lowercase();
        assert!(
            lower.contains("quick") || lower.contains("fox") || lower.contains("lazy"),
            "unexpected transcript: {transcript}"
        );
    }
}
