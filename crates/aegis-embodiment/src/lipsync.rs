//! Pure, deterministic lip-sync *hook* (not audio-synced).
//!
//! [`plan_lip_sync`] maps a transcript, and optional caller-supplied per-word
//! timings, to mouth-frame indices and a per-frame [`MouthShape`] track at a
//! requested frame rate and duration. It uses no audio, microphone, speech
//! model, or wall clock, and it is **not integrated with live audio**. The
//! mapping is a deterministic text heuristic: words are distributed across the
//! duration (or placed at the supplied timings), each word is assigned a mouth
//! shape from a small cycle, and frames not covered by a word are [`MouthShape::Closed`].
//!
//! This is a hook a future audio-synced animator could replace; it does not claim
//! to be phoneme or viseme accurate. [`crate::sequence::render_state_sequence`]
//! accepts a [`LipSyncPlan`] and paints each frame's [`MouthShape`] into the
//! schematic mouth region, but the plan itself is always derived from text and
//! optional caller timings, never from audio analysis.

use serde::{Deserialize, Serialize};

use crate::error::EmbodimentError;
use crate::sequence::sequence_frame_count;

/// Schema identifier for a lip-sync plan.
pub const LIPSYNC_SCHEMA: &str = "aegis.embodiment.lipsync.v1";
/// Maximum number of words accepted in one transcript.
pub const MAX_LIPSYNC_WORDS: usize = 4_096;
/// Maximum number of characters accepted in one transcript.
pub const MAX_LIPSYNC_TRANSCRIPT_CHARS: usize = 65_536;

/// Schematic mouth shapes. These are not phonemes or visemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouthShape {
    Closed,
    Slight,
    Open,
    Wide,
}

impl MouthShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Slight => "slight",
            Self::Open => "open",
            Self::Wide => "wide",
        }
    }
}

/// Optional caller-supplied timing for one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTiming {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl WordTiming {
    pub const fn new(start_ms: u64, end_ms: u64) -> Self {
        Self { start_ms, end_ms }
    }
}

/// One word mapped to a representative frame and mouth shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LipSyncCue {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Representative frame index (the word's midpoint), always `< frame_count`.
    pub frame_index: usize,
    pub mouth: MouthShape,
}

/// Deterministic transcript-to-mouth-frame plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LipSyncPlan {
    pub schema: String,
    pub fps: u32,
    pub duration_ms: u64,
    pub frame_count: usize,
    pub cues: Vec<LipSyncCue>,
    /// One shape per frame; silent frames are [`MouthShape::Closed`].
    pub mouth_frames: Vec<MouthShape>,
}

impl LipSyncPlan {
    /// Serialize to compact JSON. Identical plans produce identical output.
    pub fn to_json(&self) -> Result<String, EmbodimentError> {
        Ok(serde_json::to_string(self)?)
    }
}

/// Map a transcript (and optional per-word timings) to mouth-frame indices.
///
/// Frame rate and duration reuse the sequence bounds. When `timings` is `None`,
/// words are distributed evenly and the duration must be at least one millisecond
/// per word. When timings are supplied they must match the word count, be
/// non-empty per word, stay within the duration, and not overlap earlier words.
pub fn plan_lip_sync(
    transcript: &str,
    timings: Option<&[WordTiming]>,
    fps: u32,
    duration_ms: u64,
) -> Result<LipSyncPlan, EmbodimentError> {
    if transcript.chars().count() > MAX_LIPSYNC_TRANSCRIPT_CHARS {
        return Err(EmbodimentError::InvalidInput("lip-sync transcript"));
    }
    let words: Vec<&str> = transcript.split_whitespace().collect();
    if words.is_empty() {
        return Err(EmbodimentError::InvalidInput("lip-sync transcript"));
    }
    if words.len() > MAX_LIPSYNC_WORDS {
        return Err(EmbodimentError::InvalidInput("lip-sync word count"));
    }

    let frame_count = sequence_frame_count(duration_ms, fps)?;
    let spans = match timings {
        Some(timings) => {
            if timings.len() != words.len() {
                return Err(EmbodimentError::InvalidInput("lip-sync timings"));
            }
            validate_timings(timings, duration_ms)?;
            timings
                .iter()
                .map(|timing| (timing.start_ms, timing.end_ms))
                .collect::<Vec<_>>()
        }
        None => {
            if duration_ms < words.len() as u64 {
                return Err(EmbodimentError::InvalidInput("lip-sync duration"));
            }
            even_spans(words.len(), duration_ms)
        }
    };

    let mut cues = Vec::with_capacity(words.len());
    let mut mouth_frames = vec![MouthShape::Closed; frame_count];
    for (index, (word, (start_ms, end_ms))) in words.iter().zip(spans.iter()).enumerate() {
        let midpoint = start_ms + (end_ms - start_ms) / 2;
        let frame_index = time_to_frame(midpoint, fps, frame_count);
        let mouth = mouth_shape_for(word, index);
        cues.push(LipSyncCue {
            word: (*word).to_owned(),
            start_ms: *start_ms,
            end_ms: *end_ms,
            frame_index,
            mouth,
        });

        let first_frame = time_to_frame(*start_ms, fps, frame_count);
        let last_frame = time_to_frame(end_ms.saturating_sub(1), fps, frame_count);
        for shape in mouth_frames
            .iter_mut()
            .take(last_frame + 1)
            .skip(first_frame)
        {
            *shape = mouth;
        }
    }

    Ok(LipSyncPlan {
        schema: LIPSYNC_SCHEMA.to_owned(),
        fps,
        duration_ms,
        frame_count,
        cues,
        mouth_frames,
    })
}

fn validate_timings(timings: &[WordTiming], duration_ms: u64) -> Result<(), EmbodimentError> {
    let mut previous_end = 0u64;
    for timing in timings {
        if timing.start_ms >= timing.end_ms || timing.end_ms > duration_ms {
            return Err(EmbodimentError::InvalidInput("lip-sync timings"));
        }
        if timing.start_ms < previous_end {
            return Err(EmbodimentError::InvalidInput("lip-sync timings"));
        }
        previous_end = timing.end_ms;
    }
    Ok(())
}

fn even_spans(word_count: usize, duration_ms: u64) -> Vec<(u64, u64)> {
    let count = word_count as u64;
    (0..word_count)
        .map(|index| {
            let index = index as u64;
            (
                duration_ms * index / count,
                duration_ms * (index + 1) / count,
            )
        })
        .collect()
}

fn time_to_frame(time_ms: u64, fps: u32, frame_count: usize) -> usize {
    let frame = (time_ms.saturating_mul(u64::from(fps)) / 1000) as usize;
    frame.min(frame_count.saturating_sub(1))
}

fn mouth_shape_for(word: &str, index: usize) -> MouthShape {
    let letters = word
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    match (letters + index) % 3 {
        0 => MouthShape::Slight,
        1 => MouthShape::Open,
        _ => MouthShape::Wide,
    }
}
