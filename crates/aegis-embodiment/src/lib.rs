//! Per-Core embodiment interfaces: asset library, appearance, expressions,
//! gestures, wardrobe, and voice profile configuration.
//!
//! Assets and appearance are stored only in the owning Core's directory and are
//! bound to that Core's immutable UUID and database instance UUID. A copied
//! database is rejected. Asset content is verified by SHA-256 on read; this is
//! content integrity, not publisher authenticity.
//!
//! No renderer or audio provider is required for these interfaces. They expose
//! the stable contracts, configuration, and asset pipeline that a renderer,
//! voice provider, or GUI consumes. The [`voice`] module adds provider-neutral
//! TTS/STT traits, an owner-configured command adapter, a Windows
//! `System.Speech` (SAPI) TTS provider that writes a WAV file without playing
//! audio, and a Windows SAPI STT provider that recognizes a caller-supplied WAV
//! file. Both SAPI paths are CPU-only and need no download. STT is
//! **file-based WAV recognition**, not microphone capture: no microphone is
//! opened and no audio is played.
//!
//! The [`scene`], [`animation`], and [`render`] modules add a validated,
//! renderer-agnostic avatar scene model, a pure deterministic animation player,
//! and a headless renderer that emits a serializable scene description. The
//! [`raster`] module additionally produces real pixels: a dependency-free
//! software rasterizer composites scene layers as solid role-colored rectangles
//! and writes a 24-bit BMP image. It does not decode PNG/JPEG assets, use a GPU,
//! open a native window, or perform lip-sync.
//!
//! The [`sequence`] module renders deterministic animation-frame sequences with
//! that rasterizer and writes them with a bounded manifest; [`expression`] maps
//! a small synthetic expression/gesture set to overlay parameters; and
//! [`lipsync`] provides a pure transcript-to-mouth-frame hook. Lip-sync is **not**
//! audio-synced and is not integrated with live audio.

pub mod animation;
pub mod appearance;
pub mod asset;
pub mod error;
pub mod expression;
pub mod lipsync;
pub mod presentation;
pub mod raster;
pub mod remote;
pub mod render;
pub mod scene;
pub mod sequence;
pub mod store;
pub mod voice;

pub use animation::{
    AnimationClip, AnimationPlayer, AnimationState, Easing, Keyframe, PlaybackMode, Transform,
    ANIMATION_SCHEMA, MAX_KEYFRAMES,
};
pub use appearance::{
    is_hex_color, validate_animation_name, validate_theme, Appearance, Theme, VoiceProfile,
    AEGIS_ACCENT, AEGIS_PRIMARY, AEGIS_TERTIARY, APPEARANCE_SCHEMA, MAX_EXPRESSIONS, MAX_GESTURES,
    MAX_WARDROBE,
};
pub use asset::{
    safe_file_name, Asset, AssetAudit, AssetAuditEntry, AssetFilter, AssetKind, AssetManifest,
    AssetManifestEntry, AssetState, AuditOutcome, NewAsset, ASSET_AUDIT_SCHEMA,
    ASSET_MANIFEST_SCHEMA, ASSET_SCHEMA, MAX_ASSET_BYTES, MAX_ASSET_METADATA_BYTES,
    MAX_ASSET_NAME_CHARS, MAX_ASSET_SEARCH_LIMIT, MAX_ASSET_TAGS, MAX_MEDIA_TYPE_CHARS,
};
pub use error::EmbodimentError;
pub use expression::{
    expression_params, gesture_params, render_expression_bmp, Expression, ExpressionParams,
    Gesture, GestureParams,
};
pub use lipsync::{
    plan_lip_sync, LipSyncCue, LipSyncPlan, MouthShape, WordTiming, LIPSYNC_SCHEMA,
    MAX_LIPSYNC_TRANSCRIPT_CHARS, MAX_LIPSYNC_WORDS,
};
pub use presentation::{
    expression_for_emotion, render_presentation_frame, select_outfit, voice_projection,
    OutfitDecision, OutfitRejection, PresentationContext, PresentationFrame, VoiceProjection,
    MAX_OUTFIT_SELECTION, MAX_VOICE_RATE_PERCENT, MAX_VOICE_VOLUME_PERCENT, MIN_VOICE_RATE_PERCENT,
    MIN_VOICE_VOLUME_PERCENT, OUTFIT_DECISION_SCHEMA, PRESENTATION_FRAME_HEIGHT,
    PRESENTATION_FRAME_SCHEMA, PRESENTATION_FRAME_WIDTH,
};
pub use raster::{
    fill_rect, render_bmp, render_bmp_at, write_bmp, Rect, BMP_HEADER_BYTES, BMP_MAGIC,
    MAX_BMP_BYTES, MAX_BMP_DIMENSION, MIN_BMP_DIMENSION,
};
pub use remote::{
    hmac_sha256, new_nonce, read_frame, sha256_hex, write_frame, ArtifactRequest, ArtifactResponse,
    FetchedArtifact, RemoteArtifactClient, RemoteArtifactServer, RemoteError, ResponseStatus,
    DEFAULT_TIMEOUT_MS, MAX_ARTIFACT_FRAME_BYTES, MAX_FRAME_BYTES, MAX_NONCE_CHARS, MAX_TIMEOUT_MS,
    MIN_SECRET_BYTES, REMOTE_PROOF_DOMAIN, REMOTE_REQUEST_SCHEMA, REMOTE_RESPONSE_SCHEMA,
};
pub use render::{
    AvatarRenderer, HeadlessSceneRenderer, SceneDescription, SceneRenderer, HEADLESS_RENDERER,
    SCENE_DESCRIPTION_SCHEMA,
};
pub use scene::{
    ActiveMotion, AvatarScene, LayerRole, SceneLayer, MAX_SCENE_ASSET_CHARS, MAX_SCENE_LAYERS,
    SCENE_SCHEMA,
};
pub use sequence::{
    frame_time_ms, render_sequence, sequence_frame_count, write_sequence, write_sequence_capped,
    RenderedFrame, SequenceFrameEntry, SequenceManifest, MAX_SEQUENCE_DURATION_MS,
    MAX_SEQUENCE_FPS, MAX_SEQUENCE_FRAMES, MAX_SEQUENCE_TOTAL_BYTES, MIN_SEQUENCE_DURATION_MS,
    MIN_SEQUENCE_FPS, SEQUENCE_MANIFEST_FILE, SEQUENCE_MANIFEST_SCHEMA,
};
pub use store::{
    AssetExport, EmbodimentStore, ASSET_EVENT_SCHEMA, ASSET_EXPORT_SCHEMA,
    EMBODIMENT_APPLICATION_ID, EMBODIMENT_COMPONENT, EMBODIMENT_DB_FILE,
};
pub use voice::{
    find_executable, probe_availability, validate_audio_len, validate_tts_text, AudioFormat,
    CapabilityStatus, CommandSpec, CommandSttProvider, CommandTtsProvider, SapiSttProvider,
    SapiTtsProvider, SttProvider, SttRequest, SttResult, TtsProvider, TtsRequest, TtsResult,
    VoiceAvailability, VoiceBinding, VoiceError, DEFAULT_COMMAND_OUTPUT_BYTES,
    DEFAULT_COMMAND_TIMEOUT_MS, DEFAULT_SAPI_TIMEOUT_MS, MAX_COMMAND_ARGS, MAX_COMMAND_ARG_CHARS,
    MAX_COMMAND_OUTPUT_BYTES, MAX_COMMAND_TIMEOUT_MS, MAX_SAPI_AUDIO_BYTES,
    MAX_SAPI_TRANSCRIPT_BYTES, MAX_STT_AUDIO_BYTES, MAX_TTS_TEXT_BYTES, MAX_VOICE_ID_CHARS,
    MAX_VOICE_LANGUAGE_CHARS, SAPI_STT_PROVIDER_NAME, SAPI_TTS_PROVIDER_NAME,
    VOICE_AVAILABILITY_SCHEMA,
};
