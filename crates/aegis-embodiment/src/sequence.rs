//! Deterministic animation-frame sequence pipeline and bounded frame writer.
//!
//! [`render_sequence`] samples an [`AnimationClip`] at a fixed frame rate over a
//! requested duration and renders every frame with the existing CPU BMP
//! rasterizer. Each [`RenderedFrame`] carries its frame index, local time in
//! milliseconds, encoded 24-bit BMP bytes, and the SHA-256 of those bytes.
//!
//! [`write_sequence`] persists a frame sequence as `frame-NNNNNN.bmp` files plus
//! a JSON manifest, refusing to write if the declared frames exceed a total-size
//! cap. [`render_state_sequence`] / [`write_state_sequence`] add the
//! emotion/world/activity-driven [`AvatarState`] path, including optional
//! lip-sync mouth shapes recorded in the manifest. Both paths are pure with
//! respect to timing: no wall clock, thread, GPU, audio, or native window is
//! involved. Output is CPU-composited BMP pixels, not decoded art or video.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::animation::{AnimationClip, AnimationPlayer, AvatarState};
use crate::error::EmbodimentError;
use crate::expression::{render_state_bmp, StateOverlay};
use crate::lipsync::{LipSyncPlan, MouthShape, LIPSYNC_SCHEMA};
use crate::raster::{render_bmp_at, write_bmp};
use crate::remote::sha256_hex;
use crate::scene::AvatarScene;

/// Smallest frame rate accepted by [`render_sequence`] / [`write_sequence`].
pub const MIN_SEQUENCE_FPS: u32 = 1;
/// Largest frame rate accepted by [`render_sequence`] / [`write_sequence`].
pub const MAX_SEQUENCE_FPS: u32 = 120;
/// Smallest sequence duration in milliseconds.
pub const MIN_SEQUENCE_DURATION_MS: u64 = 1;
/// Largest sequence duration in milliseconds (one hour).
pub const MAX_SEQUENCE_DURATION_MS: u64 = 60 * 60 * 1000;
/// Largest number of frames a single sequence may contain.
pub const MAX_SEQUENCE_FRAMES: usize = 7_200;
/// Default total-size cap for a written sequence (256 MiB).
pub const MAX_SEQUENCE_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// Schema identifier for a sequence manifest.
pub const SEQUENCE_MANIFEST_SCHEMA: &str = "aegis.embodiment.sequence_manifest.v1";
/// File name of the JSON manifest written next to the frames.
pub const SEQUENCE_MANIFEST_FILE: &str = "manifest.json";

/// Timing and size parameters for a rendered sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceOptions {
    pub fps: u32,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
}

impl SequenceOptions {
    pub const fn new(fps: u32, duration_ms: u64, width: u32, height: u32) -> Self {
        Self {
            fps,
            duration_ms,
            width,
            height,
        }
    }
}

/// One rendered frame in a deterministic sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedFrame {
    /// Zero-based frame index.
    pub index: usize,
    /// Local clip time sampled for this frame, in milliseconds.
    pub time_ms: u64,
    /// Encoded 24-bit BMP bytes.
    pub bmp: Vec<u8>,
    /// Lower-case hex SHA-256 of `bmp`.
    pub sha256: String,
}

/// One manifest entry describing a written frame file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceFrameEntry {
    pub index: usize,
    pub file: String,
    pub time_ms: u64,
    pub bytes: usize,
    pub sha256: String,
}

/// Manifest written alongside a frame sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceManifest {
    pub schema: String,
    pub fps: u32,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub frame_count: usize,
    pub total_bytes: usize,
    /// Projected avatar state the frames were rendered from, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_state: Option<AvatarState>,
    /// Per-frame lip-sync mouth shapes, if a plan was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouth_frames: Option<Vec<MouthShape>>,
    pub frames: Vec<SequenceFrameEntry>,
}

impl SequenceManifest {
    /// Serialize to compact JSON. Identical manifests produce identical output.
    pub fn to_json(&self) -> Result<String, EmbodimentError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Serialize to pretty JSON. Identical manifests produce identical output.
    pub fn to_json_pretty(&self) -> Result<String, EmbodimentError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// Compute the deterministic frame count for `duration_ms` at `fps`.
///
/// Uses ceiling division so a sequence always covers the requested duration.
/// Returns [`EmbodimentError::InvalidInput`] if the rate, duration, or resulting
/// frame count is outside the documented bounds.
pub fn sequence_frame_count(duration_ms: u64, fps: u32) -> Result<usize, EmbodimentError> {
    if !(MIN_SEQUENCE_FPS..=MAX_SEQUENCE_FPS).contains(&fps) {
        return Err(EmbodimentError::InvalidInput("sequence fps"));
    }
    if !(MIN_SEQUENCE_DURATION_MS..=MAX_SEQUENCE_DURATION_MS).contains(&duration_ms) {
        return Err(EmbodimentError::InvalidInput("sequence duration"));
    }
    let numerator = duration_ms
        .checked_mul(u64::from(fps))
        .ok_or(EmbodimentError::InvalidInput("sequence duration"))?;
    let frames = usize::try_from(numerator.div_ceil(1000))
        .map_err(|_| EmbodimentError::InvalidInput("sequence frame count"))?;
    if frames == 0 || frames > MAX_SEQUENCE_FRAMES {
        return Err(EmbodimentError::InvalidInput("sequence frame count"));
    }
    Ok(frames)
}

/// Deterministic local time of `index` for a sequence rendered at `fps`.
pub fn frame_time_ms(index: usize, fps: u32) -> u64 {
    (index as u64).saturating_mul(1000) / u64::from(fps.max(1))
}

/// Render a deterministic animation-frame sequence using the CPU BMP rasterizer.
///
/// Validates the clip and timing bounds, then samples the clip at each frame
/// time via [`AnimationPlayer::seek`] and renders with [`render_bmp_at`]. The
/// same scene, clip, size, duration, and rate always produce the same bytes and
/// digests.
pub fn render_sequence(
    scene: &AvatarScene,
    clip: &AnimationClip,
    width: u32,
    height: u32,
    duration_ms: u64,
    fps: u32,
) -> Result<Vec<RenderedFrame>, EmbodimentError> {
    clip.validate()?;
    let frame_count = sequence_frame_count(duration_ms, fps)?;
    let mut player = AnimationPlayer::new(clip.clone());
    let mut frames = Vec::with_capacity(frame_count);
    for index in 0..frame_count {
        let time_ms = frame_time_ms(index, fps);
        let state = player.seek(time_ms);
        let bmp = render_bmp_at(scene, width, height, Some(&state))?;
        let sha256 = sha256_hex(&bmp);
        frames.push(RenderedFrame {
            index,
            time_ms,
            bmp,
            sha256,
        });
    }
    Ok(frames)
}

/// Render a deterministic frame sequence that reflects an [`AvatarState`].
///
/// Each frame samples [`AvatarState::clip`] and renders the base scene with the
/// resulting animation transform, then overlays the state expression, gesture,
/// and (when `lipsync` is supplied) that frame's mouth shape. The lip-sync plan
/// must match this sequence's frame rate, duration, and frame count exactly.
///
/// If the state carries a Core UUID it must equal `scene.core_uuid`, so a state
/// projected for one Core can never render another Core's scene. This is still
/// schematic CPU BMP output, not art decoding, audio analysis, a GPU renderer,
/// or a native window.
pub fn render_state_sequence(
    scene: &AvatarScene,
    state: &AvatarState,
    lipsync: Option<&LipSyncPlan>,
    options: SequenceOptions,
) -> Result<Vec<RenderedFrame>, EmbodimentError> {
    scene.validate()?;
    state.validate()?;
    if let Some(core_uuid) = &state.core_uuid {
        if core_uuid != &scene.core_uuid {
            return Err(EmbodimentError::OwnershipMismatch);
        }
    }
    let frame_count = sequence_frame_count(options.duration_ms, options.fps)?;
    if let Some(plan) = lipsync {
        validate_lipsync(plan, options, frame_count)?;
    }
    let clip = state.clip()?;
    let mut player = AnimationPlayer::new(clip);
    let mut frames = Vec::with_capacity(frame_count);
    for index in 0..frame_count {
        let time_ms = frame_time_ms(index, options.fps);
        let animation = player.seek(time_ms);
        let mouth = lipsync.map(|plan| plan.mouth_frames[index]);
        let overlay = StateOverlay::new(state.expression, state.gesture, mouth);
        let bmp = render_state_bmp(
            scene,
            &overlay,
            options.width,
            options.height,
            Some(&animation),
        )?;
        let sha256 = sha256_hex(&bmp);
        frames.push(RenderedFrame {
            index,
            time_ms,
            bmp,
            sha256,
        });
    }
    Ok(frames)
}

fn validate_lipsync(
    plan: &LipSyncPlan,
    options: SequenceOptions,
    frame_count: usize,
) -> Result<(), EmbodimentError> {
    if plan.schema != LIPSYNC_SCHEMA {
        return Err(EmbodimentError::CorruptData("lip-sync schema"));
    }
    if plan.fps != options.fps || plan.duration_ms != options.duration_ms {
        return Err(EmbodimentError::InvalidInput("lip-sync timing"));
    }
    if plan.frame_count != frame_count || plan.mouth_frames.len() != frame_count {
        return Err(EmbodimentError::InvalidInput("lip-sync frame count"));
    }
    Ok(())
}

/// Write a frame sequence with the default [`MAX_SEQUENCE_TOTAL_BYTES`] cap.
pub fn write_sequence(
    dir: impl AsRef<Path>,
    frames: &[RenderedFrame],
    fps: u32,
    duration_ms: u64,
    width: u32,
    height: u32,
) -> Result<SequenceManifest, EmbodimentError> {
    write_sequence_capped(
        dir,
        frames,
        fps,
        duration_ms,
        width,
        height,
        MAX_SEQUENCE_TOTAL_BYTES,
    )
}

/// Write a frame sequence to `dir` without exceeding `max_total_bytes`.
///
/// The total size is computed and checked before any file is written. Every
/// frame digest is re-verified and every BMP is validated by [`write_bmp`].
/// Frame files are named `frame-NNNNNN.bmp`; the manifest is written last as
/// [`SEQUENCE_MANIFEST_FILE`].
pub fn write_sequence_capped(
    dir: impl AsRef<Path>,
    frames: &[RenderedFrame],
    fps: u32,
    duration_ms: u64,
    width: u32,
    height: u32,
    max_total_bytes: usize,
) -> Result<SequenceManifest, EmbodimentError> {
    write_sequence_inner(
        dir,
        frames,
        SequenceOptions::new(fps, duration_ms, width, height),
        max_total_bytes,
        None,
        None,
    )
}

/// Write a state-aware frame sequence with the default size cap.
pub fn write_state_sequence(
    dir: impl AsRef<Path>,
    frames: &[RenderedFrame],
    state: &AvatarState,
    lipsync: Option<&LipSyncPlan>,
    options: SequenceOptions,
) -> Result<SequenceManifest, EmbodimentError> {
    write_state_sequence_capped(
        dir,
        frames,
        state,
        lipsync,
        options,
        MAX_SEQUENCE_TOTAL_BYTES,
    )
}

/// Write a state-aware frame sequence without exceeding `max_total_bytes`.
///
/// The manifest additionally records the projected [`AvatarState`] and, when a
/// lip-sync plan is supplied, its per-frame mouth shapes. Frame timing, frame
/// count, and lip-sync duration must all agree before any file is written.
pub fn write_state_sequence_capped(
    dir: impl AsRef<Path>,
    frames: &[RenderedFrame],
    state: &AvatarState,
    lipsync: Option<&LipSyncPlan>,
    options: SequenceOptions,
    max_total_bytes: usize,
) -> Result<SequenceManifest, EmbodimentError> {
    state.validate()?;
    let expected = sequence_frame_count(options.duration_ms, options.fps)?;
    if frames.len() != expected {
        return Err(EmbodimentError::InvalidInput("sequence frame count"));
    }
    if let Some(plan) = lipsync {
        validate_lipsync(plan, options, frames.len())?;
    }
    write_sequence_inner(
        dir,
        frames,
        options,
        max_total_bytes,
        Some(state),
        lipsync.map(|plan| plan.mouth_frames.as_slice()),
    )
}

fn write_sequence_inner(
    dir: impl AsRef<Path>,
    frames: &[RenderedFrame],
    options: SequenceOptions,
    max_total_bytes: usize,
    avatar_state: Option<&AvatarState>,
    mouth_frames: Option<&[MouthShape]>,
) -> Result<SequenceManifest, EmbodimentError> {
    let expected = sequence_frame_count(options.duration_ms, options.fps)?;
    if frames.len() != expected {
        return Err(EmbodimentError::InvalidInput("sequence frame count"));
    }
    if let Some(state) = avatar_state {
        state.validate()?;
    }
    if let Some(shape_frames) = mouth_frames {
        if shape_frames.len() != frames.len() {
            return Err(EmbodimentError::InvalidInput("sequence mouth frames"));
        }
    }

    let mut total_bytes = 0usize;
    let mut entries = Vec::with_capacity(frames.len());
    for (position, frame) in frames.iter().enumerate() {
        if frame.index != position {
            return Err(EmbodimentError::CorruptData("sequence frame index"));
        }
        if sha256_hex(&frame.bmp) != frame.sha256 {
            return Err(EmbodimentError::CorruptData("sequence frame digest"));
        }
        total_bytes = total_bytes
            .checked_add(frame.bmp.len())
            .ok_or(EmbodimentError::InvalidInput("sequence total bytes"))?;
        entries.push(SequenceFrameEntry {
            index: frame.index,
            file: frame_file_name(frame.index),
            time_ms: frame.time_ms,
            bytes: frame.bmp.len(),
            sha256: frame.sha256.clone(),
        });
    }
    if total_bytes > max_total_bytes {
        return Err(EmbodimentError::InvalidInput("sequence total bytes"));
    }

    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)?;
    for (frame, entry) in frames.iter().zip(entries.iter()) {
        write_bmp(dir.join(&entry.file), &frame.bmp)?;
    }

    let manifest = SequenceManifest {
        schema: SEQUENCE_MANIFEST_SCHEMA.to_owned(),
        fps: options.fps,
        duration_ms: options.duration_ms,
        width: options.width,
        height: options.height,
        frame_count: frames.len(),
        total_bytes,
        avatar_state: avatar_state.cloned(),
        mouth_frames: mouth_frames.map(|shapes| shapes.to_vec()),
        frames: entries,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(dir.join(SEQUENCE_MANIFEST_FILE), manifest_bytes)?;
    Ok(manifest)
}

fn frame_file_name(index: usize) -> String {
    format!("frame-{index:06}.bmp")
}
