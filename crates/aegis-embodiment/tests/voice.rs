use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::voice::{
    TtsStyle, MAX_TTS_RATE_PERCENT, MAX_TTS_VOLUME_PERCENT, MIN_TTS_RATE_PERCENT,
    MIN_TTS_VOLUME_PERCENT,
};
use aegis_embodiment::{
    probe_availability, validate_audio_len, validate_tts_text, voice_projection, AssetKind,
    AudioFormat, CommandSpec, CommandSttProvider, CommandTtsProvider, EmbodimentError,
    EmbodimentStore, NewAsset, SapiTtsProvider, SttProvider, SttRequest, TtsProvider, TtsRequest,
    VoiceError, VoiceProfile, DEFAULT_COMMAND_OUTPUT_BYTES, MAX_COMMAND_ARGS,
    MAX_COMMAND_ARG_CHARS, MAX_STT_AUDIO_BYTES, MAX_TTS_TEXT_BYTES, VOICE_AVAILABILITY_SCHEMA,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const MOCK: &str = env!("CARGO_BIN_EXE_voice_mock");

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Sofía Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
        installer
            .initialize(
                &CoreInitializationRequest {
                    schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                    core_uuid: uuid.to_owned(),
                    core_number: number,
                    display_name: name.to_owned(),
                    requested_at: now_rfc3339().expect("time"),
                },
                &initialized.owner_token,
            )
            .expect("core");
    }
    (temp, root)
}

fn open_store(root: &Path, core: &str) -> EmbodimentStore {
    EmbodimentStore::open(root, core).expect("store")
}

fn import(
    store: &EmbodimentStore,
    kind: AssetKind,
    name: &str,
    media_type: &str,
    bytes: &[u8],
) -> String {
    store
        .import_asset(NewAsset::new(kind, name, media_type, bytes))
        .expect("import")
        .asset_id
}

fn mock_spec(mode: &str) -> CommandSpec {
    CommandSpec::new(MOCK)
        .with_arg(mode)
        .with_timeout(Duration::from_secs(10))
}

#[test]
fn tts_request_validation_rejects_empty_and_oversized_text() {
    assert!(matches!(
        TtsRequest::new("", AudioFormat::Wav).validate(),
        Err(VoiceError::EmptyText)
    ));
    assert!(matches!(
        TtsRequest::new("   \t\n", AudioFormat::Wav).validate(),
        Err(VoiceError::EmptyText)
    ));
    assert!(matches!(validate_tts_text("hello"), Ok(())));

    let oversized = "x".repeat(MAX_TTS_TEXT_BYTES + 1);
    assert!(matches!(
        TtsRequest::new(&oversized, AudioFormat::Mp3).validate(),
        Err(VoiceError::TextTooLarge { .. })
    ));
    let exact = "y".repeat(MAX_TTS_TEXT_BYTES);
    assert!(TtsRequest::new(&exact, AudioFormat::Mp3).validate().is_ok());
}

#[test]
fn tts_request_validation_rejects_invalid_language_and_voice() {
    let bad_language = TtsRequest::new("hello", AudioFormat::Wav).with_language("\n");
    assert!(matches!(
        bad_language.validate(),
        Err(VoiceError::InvalidLanguage)
    ));

    let empty_language = TtsRequest::new("hello", AudioFormat::Wav).with_language("   ");
    assert!(matches!(
        empty_language.validate(),
        Err(VoiceError::InvalidLanguage)
    ));

    let bad_voice = VoiceProfile {
        voice_id: String::new(),
        display_name: "Voice".to_owned(),
        sample_asset_id: None,
    };
    let with_bad_voice = TtsRequest::new("hello", AudioFormat::Wav).with_voice(&bad_voice);
    assert!(matches!(
        with_bad_voice.validate(),
        Err(VoiceError::InvalidVoiceProfile)
    ));

    let empty_sample = VoiceProfile {
        voice_id: "voice".to_owned(),
        display_name: "Voice".to_owned(),
        sample_asset_id: Some("   ".to_owned()),
    };
    let with_empty_sample = TtsRequest::new("hello", AudioFormat::Wav).with_voice(&empty_sample);
    assert!(matches!(
        with_empty_sample.validate(),
        Err(VoiceError::InvalidVoiceProfile)
    ));

    let good_voice = VoiceProfile {
        voice_id: "voice".to_owned(),
        display_name: "Voice".to_owned(),
        sample_asset_id: Some("asset-id".to_owned()),
    };
    let good = TtsRequest::new("hello", AudioFormat::Wav)
        .with_voice(&good_voice)
        .with_language("en-US");
    assert!(good.validate().is_ok());
}

#[test]
fn tts_style_defaults_preserve_legacy_behavior_and_bounds() {
    assert_eq!(TtsStyle::default(), TtsStyle::new(100, 100));

    let legacy = TtsRequest::new("hello", AudioFormat::Wav);
    assert!(legacy.style.is_none());
    assert!(legacy.validate().is_ok());

    assert!(TtsStyle::new(MIN_TTS_RATE_PERCENT, MIN_TTS_VOLUME_PERCENT)
        .validate()
        .is_ok());
    assert!(TtsStyle::new(MAX_TTS_RATE_PERCENT, MAX_TTS_VOLUME_PERCENT)
        .validate()
        .is_ok());

    let slow = TtsRequest::new("hello", AudioFormat::Wav)
        .with_style_percent(MIN_TTS_RATE_PERCENT - 1, 100);
    assert!(matches!(
        slow.validate(),
        Err(VoiceError::InvalidRatePercent { .. })
    ));

    let fast = TtsRequest::new("hello", AudioFormat::Wav)
        .with_style_percent(MAX_TTS_RATE_PERCENT + 1, 100);
    assert!(matches!(
        fast.validate(),
        Err(VoiceError::InvalidRatePercent { .. })
    ));

    let loud = TtsRequest::new("hello", AudioFormat::Wav)
        .with_style_percent(100, MAX_TTS_VOLUME_PERCENT + 1);
    assert!(matches!(
        loud.validate(),
        Err(VoiceError::InvalidVolumePercent { .. })
    ));
}

#[test]
fn tts_style_can_be_derived_from_voice_projection() {
    let neutral = voice_projection(None);
    let style = TtsStyle::from_projection(&neutral);
    assert_eq!(style, TtsStyle::default());
    assert!(style.validate().is_ok());

    let request = TtsRequest::new("hello", AudioFormat::Wav).with_projection(&neutral);
    assert_eq!(request.style, Some(TtsStyle::default()));
    assert!(request.validate().is_ok());

    let provider = SapiTtsProvider::system_default().with_projection(&neutral);
    assert_eq!(provider.style(), Some(TtsStyle::default()));
}

#[test]
fn sapi_tts_rejects_out_of_bounds_style_before_synthesis() {
    let request = TtsRequest::new("hello", AudioFormat::Wav)
        .with_style_percent(MAX_TTS_RATE_PERCENT + 1, 100);
    assert!(matches!(
        SapiTtsProvider::system_default().synthesize(&request),
        Err(VoiceError::InvalidRatePercent { .. })
    ));

    let request = TtsRequest::new("hello", AudioFormat::Wav)
        .with_style_percent(100, MAX_TTS_VOLUME_PERCENT + 1);
    assert!(matches!(
        SapiTtsProvider::system_default().synthesize(&request),
        Err(VoiceError::InvalidVolumePercent { .. })
    ));

    let provider =
        SapiTtsProvider::system_default().with_style_percent(MIN_TTS_RATE_PERCENT - 1, 100);
    assert!(matches!(
        provider.synthesize(&TtsRequest::new("hello", AudioFormat::Wav)),
        Err(VoiceError::InvalidRatePercent { .. })
    ));
}

#[test]
fn stt_request_validation_rejects_empty_and_oversized_audio() {
    assert!(matches!(
        SttRequest::new(b"", AudioFormat::Wav).validate(),
        Err(VoiceError::EmptyAudio)
    ));
    assert!(matches!(validate_audio_len(0), Err(VoiceError::EmptyAudio)));
    assert!(matches!(
        validate_audio_len(MAX_STT_AUDIO_BYTES + 1),
        Err(VoiceError::AudioTooLarge { .. })
    ));
    assert!(validate_audio_len(MAX_STT_AUDIO_BYTES).is_ok());

    let good = SttRequest::new(b"audio-bytes", AudioFormat::Pcm16Le).with_language("en");
    assert!(good.validate().is_ok());
    let bad = SttRequest::new(b"audio-bytes", AudioFormat::Pcm16Le).with_language("\u{0}");
    assert!(matches!(bad.validate(), Err(VoiceError::InvalidLanguage)));
}

#[test]
fn command_spec_validation_enforces_bounds() {
    assert!(matches!(
        CommandSpec::new("").validate(),
        Err(VoiceError::EmptyCommand)
    ));

    let too_many = (0..=MAX_COMMAND_ARGS).map(|index| index.to_string());
    assert!(matches!(
        CommandSpec::new(MOCK).with_args(too_many).validate(),
        Err(VoiceError::TooManyArguments { .. })
    ));

    let long_arg = "x".repeat(MAX_COMMAND_ARG_CHARS + 1);
    assert!(matches!(
        CommandSpec::new(MOCK).with_arg(long_arg).validate(),
        Err(VoiceError::ArgumentTooLong { .. })
    ));

    assert!(matches!(
        CommandSpec::new(MOCK).with_arg("a\0b").validate(),
        Err(VoiceError::InvalidArgument { .. })
    ));
    assert!(matches!(
        CommandSpec::new(MOCK)
            .with_timeout(Duration::from_millis(0))
            .validate(),
        Err(VoiceError::InvalidTimeout { .. })
    ));
    assert!(matches!(
        CommandSpec::new(MOCK).with_max_output_bytes(0).validate(),
        Err(VoiceError::InvalidOutputLimit { .. })
    ));
    assert!(CommandSpec::new(MOCK)
        .with_arg("echo")
        .with_timeout(Duration::from_secs(1))
        .with_max_output_bytes(1024)
        .validate()
        .is_ok());
}

#[test]
fn command_tts_provider_round_trips_through_mock_executable() {
    let provider = CommandTtsProvider::new("mock-tts", mock_spec("echo"));
    let voice = VoiceProfile {
        voice_id: "voice-a".to_owned(),
        display_name: "Voice A".to_owned(),
        sample_asset_id: None,
    };
    let request = TtsRequest::new("hello voice", AudioFormat::Wav).with_voice(&voice);
    let result = provider.synthesize(&request).expect("synthesize");

    assert_eq!(result.audio, b"hello voice");
    assert_eq!(result.format, AudioFormat::Wav);
    assert_eq!(result.media_type, "audio/wav");
    assert_eq!(result.provider, "mock-tts");
    assert_eq!(result.voice_id.as_deref(), Some("voice-a"));
    assert_eq!(provider.name(), "mock-tts");
    assert_eq!(
        provider.spec().max_output_bytes(),
        DEFAULT_COMMAND_OUTPUT_BYTES
    );
}

#[test]
fn command_stt_provider_round_trips_through_mock_executable() {
    let provider = CommandSttProvider::new("mock-stt", mock_spec("echo"));
    let request = SttRequest::new(b"hello transcript", AudioFormat::Pcm16Le).with_language("en-US");
    let result = provider.transcribe(&request).expect("transcribe");

    assert_eq!(result.transcript, "hello transcript");
    assert_eq!(result.format, AudioFormat::Pcm16Le);
    assert_eq!(result.provider, "mock-stt");
    assert_eq!(result.language.as_deref(), Some("en-US"));
    assert_eq!(provider.name(), "mock-stt");
}

#[test]
fn command_provider_maps_timeout() {
    let spec = CommandSpec::new(MOCK)
        .with_arg("sleep")
        .with_arg("5000")
        .with_timeout(Duration::from_millis(100));
    let provider = CommandTtsProvider::new("slow", spec);
    let error = provider
        .synthesize(&TtsRequest::new("hello", AudioFormat::Wav))
        .expect_err("timeout");
    assert!(matches!(error, VoiceError::Timeout { timeout_ms: 100 }));
}

#[test]
fn command_provider_maps_nonzero_exit() {
    let provider = CommandTtsProvider::new("failing", mock_spec("fail"));
    let error = provider
        .synthesize(&TtsRequest::new("hello", AudioFormat::Wav))
        .expect_err("non-zero exit");
    match error {
        VoiceError::NonZeroExit { status, stderr } => {
            assert!(!status.is_empty());
            assert!(stderr.contains("voice mock failure"), "stderr: {stderr}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn command_provider_enforces_output_cap() {
    let spec = CommandSpec::new(MOCK)
        .with_arg("big")
        .with_arg("4096")
        .with_max_output_bytes(100)
        .with_timeout(Duration::from_secs(10));
    let provider = CommandTtsProvider::new("large", spec);
    let error = provider
        .synthesize(&TtsRequest::new("hello", AudioFormat::Wav))
        .expect_err("output cap");
    assert!(matches!(error, VoiceError::OutputTooLarge { limit: 100 }));
}

#[test]
fn command_tts_provider_rejects_empty_output() {
    let spec = CommandSpec::new(MOCK)
        .with_arg("sleep")
        .with_arg("0")
        .with_timeout(Duration::from_secs(10));
    let provider = CommandTtsProvider::new("empty", spec);
    let error = provider
        .synthesize(&TtsRequest::new("hello", AudioFormat::Wav))
        .expect_err("empty output");
    assert!(matches!(error, VoiceError::EmptyOutput));
}

#[test]
fn availability_probe_reports_unavailable_with_reasons() {
    let availability = probe_availability();
    assert_eq!(availability.schema, VOICE_AVAILABILITY_SCHEMA);
    assert!(!availability.tts.available);
    assert!(!availability.stt.available);
    assert!(!availability.tts.verified);
    assert!(!availability.stt.verified);
    assert!(!availability.is_available());
    assert!(availability.tts.reason.contains("TTS"));
    assert!(availability.stt.reason.contains("STT"));
    assert!(availability.tts.reason.contains("live audio"));
    assert!(availability.stt.reason.contains("live audio"));

    let json = serde_json::to_string(&availability).expect("serialize");
    assert!(json.contains(VOICE_AVAILABILITY_SCHEMA));
}

#[test]
fn voice_binding_is_absent_when_unset() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    assert!(store.validate_voice_binding().expect("binding").is_none());
    assert!(store.voice_binding().expect("binding alias").is_none());
}

#[test]
fn cores_keep_distinct_voice_configs_and_isolated_samples() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);
    let sample_a = import(
        &store_a,
        AssetKind::VoiceSample,
        "a.wav",
        "audio/wav",
        b"voice-a",
    );
    let sample_b = import(
        &store_b,
        AssetKind::VoiceSample,
        "b.wav",
        "audio/wav",
        b"voice-b",
    );

    store_a
        .set_voice(Some(VoiceProfile {
            voice_id: "voice-a".to_owned(),
            display_name: "Voice A".to_owned(),
            sample_asset_id: Some(sample_a.clone()),
        }))
        .expect("voice a");
    store_b
        .set_voice(Some(VoiceProfile {
            voice_id: "voice-b".to_owned(),
            display_name: "Voice B".to_owned(),
            sample_asset_id: Some(sample_b.clone()),
        }))
        .expect("voice b");

    let binding_a = store_a
        .validate_voice_binding()
        .expect("binding a")
        .expect("voice a");
    let binding_b = store_b
        .validate_voice_binding()
        .expect("binding b")
        .expect("voice b");
    assert_eq!(binding_a.core_uuid, CORE_A);
    assert_eq!(binding_b.core_uuid, CORE_B);
    assert_eq!(binding_a.voice_id, "voice-a");
    assert_eq!(binding_b.voice_id, "voice-b");
    assert_ne!(binding_a.voice_id, binding_b.voice_id);
    assert_eq!(
        binding_a.sample_asset_id.as_deref(),
        Some(sample_a.as_str())
    );
    assert_eq!(
        binding_b.sample_asset_id.as_deref(),
        Some(sample_b.as_str())
    );
    assert_ne!(binding_a.sample_asset_id, binding_b.sample_asset_id);
    assert_eq!(binding_a.sample_bytes, Some(7));
    assert_eq!(binding_b.sample_bytes, Some(7));
    assert_eq!(binding_a.sample_sha256.as_deref().map(str::len), Some(64));
    assert_eq!(binding_b.sample_sha256.as_deref().map(str::len), Some(64));
    assert_ne!(binding_a.sample_sha256, binding_b.sample_sha256);

    assert!(matches!(
        store_a.read_asset(&sample_b),
        Err(EmbodimentError::NotFound)
    ));
    assert!(matches!(
        store_b.read_asset(&sample_a),
        Err(EmbodimentError::NotFound)
    ));
    assert!(matches!(
        store_b.asset(&sample_a),
        Err(EmbodimentError::NotFound)
    ));

    let stolen = store_b
        .set_voice(Some(VoiceProfile {
            voice_id: "stolen".to_owned(),
            display_name: "Stolen".to_owned(),
            sample_asset_id: Some(sample_a),
        }))
        .expect_err("foreign sample must be rejected");
    assert!(matches!(stolen, EmbodimentError::NotFound));
    assert_eq!(
        store_b
            .validate_voice_binding()
            .expect("binding b unchanged")
            .expect("voice b")
            .voice_id,
        "voice-b"
    );
}

#[test]
fn tts_request_for_core_a_cannot_use_core_b_voice_sample() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);
    let sample_a = import(
        &store_a,
        AssetKind::VoiceSample,
        "a.wav",
        "audio/wav",
        b"core-a-voice",
    );
    let sample_b = import(
        &store_b,
        AssetKind::VoiceSample,
        "b.wav",
        "audio/wav",
        b"core-b-voice",
    );

    let foreign = store_a
        .set_voice(Some(VoiceProfile {
            voice_id: "core-b-voice".to_owned(),
            display_name: "Core B Voice".to_owned(),
            sample_asset_id: Some(sample_b.clone()),
        }))
        .expect_err("Core A must not bind Core B's sample asset");
    assert!(matches!(foreign, EmbodimentError::NotFound));
    assert!(store_a.voice_binding().expect("binding").is_none());

    store_a
        .set_voice(Some(VoiceProfile {
            voice_id: "core-a-voice".to_owned(),
            display_name: "Core A Voice".to_owned(),
            sample_asset_id: Some(sample_a.clone()),
        }))
        .expect("bind core a sample");
    let binding = store_a
        .voice_binding()
        .expect("binding")
        .expect("core a voice");
    assert_eq!(binding.sample_asset_id.as_deref(), Some(sample_a.as_str()));
    assert_ne!(binding.sample_asset_id.as_deref(), Some(sample_b.as_str()));

    let profile = VoiceProfile {
        voice_id: binding.voice_id.clone(),
        display_name: binding.display_name.clone(),
        sample_asset_id: binding.sample_asset_id.clone(),
    };
    let request = TtsRequest::new("hello from core a", AudioFormat::Wav).with_voice(&profile);
    assert!(request.validate().is_ok());
    assert_eq!(
        store_a.read_asset(&sample_a).expect("read a"),
        b"core-a-voice"
    );
    assert!(matches!(
        store_a.read_asset(&sample_b),
        Err(EmbodimentError::NotFound)
    ));
}

#[test]
fn stt_transcript_for_core_a_does_not_cross_into_core_b() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);
    let audio_a = import(
        &store_a,
        AssetKind::VoiceSample,
        "a.wav",
        "audio/wav",
        b"core a transcript",
    );
    let audio_b = import(
        &store_b,
        AssetKind::VoiceSample,
        "b.wav",
        "audio/wav",
        b"core b transcript",
    );

    let provider = CommandSttProvider::new("mock-stt", mock_spec("echo"));

    let audio_a_bytes = store_a.read_asset(&audio_a).expect("read core a audio");
    let stt_a = provider
        .transcribe(&SttRequest::new(&audio_a_bytes, AudioFormat::Pcm16Le))
        .expect("transcribe core a");
    let audio_b_bytes = store_b.read_asset(&audio_b).expect("read core b audio");
    let stt_b = provider
        .transcribe(&SttRequest::new(&audio_b_bytes, AudioFormat::Pcm16Le))
        .expect("transcribe core b");

    assert_eq!(stt_a.transcript, "core a transcript");
    assert_eq!(stt_b.transcript, "core b transcript");
    assert_ne!(stt_a.transcript, stt_b.transcript);

    assert!(matches!(
        store_b.read_asset(&audio_a),
        Err(EmbodimentError::NotFound)
    ));
    assert!(matches!(
        store_a.read_asset(&audio_b),
        Err(EmbodimentError::NotFound)
    ));

    assert!(store_a.voice_binding().expect("binding a").is_none());
    assert!(store_b.voice_binding().expect("binding b").is_none());
}

#[test]
fn voice_binding_rejects_wrong_kind_and_detects_tampering() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let avatar = import(
        &store,
        AssetKind::Avatar,
        "base.png",
        "image/png",
        b"avatar",
    );
    let wrong_kind = store
        .set_voice(Some(VoiceProfile {
            voice_id: "voice-a".to_owned(),
            display_name: "Voice A".to_owned(),
            sample_asset_id: Some(avatar),
        }))
        .expect_err("wrong kind");
    assert!(matches!(wrong_kind, EmbodimentError::WrongAssetKind(_)));

    let sample = import(
        &store,
        AssetKind::VoiceSample,
        "a.wav",
        "audio/wav",
        b"voice-a",
    );
    store
        .set_voice(Some(VoiceProfile {
            voice_id: "voice-a".to_owned(),
            display_name: "Voice A".to_owned(),
            sample_asset_id: Some(sample.clone()),
        }))
        .expect("voice");
    assert!(store.validate_voice_binding().expect("binding").is_some());

    let relative = store.asset(&sample).expect("asset").relative_path;
    let path = root.join("cores").join(CORE_A).join(relative);
    fs::write(&path, b"tampered").expect("tamper");
    assert!(matches!(
        store.validate_voice_binding(),
        Err(EmbodimentError::IntegrityMismatch)
    ));

    store.delete_asset(&sample).expect("delete");
    assert!(matches!(
        store.validate_voice_binding(),
        Err(EmbodimentError::Deleted)
    ));
}

#[cfg(windows)]
#[test]
fn live_sapi_tts_synthesizes_nondefault_rate_when_enabled() {
    if std::env::var("AEGIS_VOICE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live SAPI TTS style check: set AEGIS_VOICE_LIVE=1 to enable");
        return;
    }
    let provider = SapiTtsProvider::system_default().with_timeout(Duration::from_secs(30));
    let request = TtsRequest::new(
        "Aegis style check at a slower non-default rate.",
        AudioFormat::Wav,
    )
    .with_style_percent(80, 70);
    match provider.synthesize(&request) {
        Ok(result) => {
            assert!(
                !result.audio.is_empty(),
                "live SAPI TTS style returned empty audio"
            );
            assert_eq!(&result.audio[0..4], b"RIFF");
            assert_eq!(&result.audio[8..12], b"WAVE");
            assert!(result.audio.len() > 44);
            eprintln!(
                "live SAPI TTS non-default style (rate 80%, volume 70%) produced {} WAV bytes",
                result.audio.len()
            );
        }
        Err(VoiceError::SapiUnavailable { reason }) => {
            eprintln!("skipping live SAPI TTS style check: SAPI unavailable: {reason}");
        }
        Err(other) => panic!("live SAPI TTS style check failed: {other}"),
    }
}
