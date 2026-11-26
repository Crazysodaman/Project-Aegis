//! Deterministic avatar animation model.
//!
//! An [`AnimationClip`] is an ordered list of keyframes carrying numeric
//! transform targets and an [`Easing`] curve for the segment that starts at that
//! keyframe. [`AnimationPlayer`] advances by elapsed milliseconds and reports a
//! clamped (once) or wrapped (looping) [`AnimationState`]. The player is pure:
//! it reads no wall clock, spawns no thread, and performs no rendering, so a
//! given sequence of `advance` calls always produces the same state.
//!
//! [`project_avatar_state`] additionally maps optional emotion, activity, and
//! world snapshots to an [`AvatarState`] with an expression, gesture, motion
//! mode, and a deterministic generated clip. Missing inputs never fabricate a
//! signal.

use serde::{Deserialize, Serialize};

use aegis_contracts::personal::{Activity, ActivityStatus, EmotionState};
use aegis_contracts::world::{Season, TimeOfDay, WorldSnapshot};

use crate::appearance::validate_animation_name;
use crate::error::EmbodimentError;
use crate::expression::{Expression, Gesture};

/// Schema identifier for an animation clip.
pub const ANIMATION_SCHEMA: &str = "aegis.embodiment.animation.v1";
/// Schema identifier for a projected avatar state.
pub const AVATAR_STATE_SCHEMA: &str = "aegis.embodiment.avatar_state.v1";
/// Maximum number of keyframes in one clip.
pub const MAX_KEYFRAMES: usize = 1024;
/// Maximum value of [`AvatarState::energy`].
pub const MAX_AVATAR_ENERGY: u8 = 100;

/// Interpolation curve applied between two keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Holds the starting value until the last instant of the segment.
    Step,
}

impl Easing {
    /// Apply the curve to a normalized progress value, clamped to `0..=1`.
    pub fn apply(self, progress: f32) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => {
                let inverse = 1.0 - t;
                1.0 - inverse * inverse
            }
            Self::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    let inverse = -2.0 * t + 2.0;
                    1.0 - inverse * inverse / 2.0
                }
            }
            Self::Step => {
                if t < 1.0 {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }
}

/// Numeric transform targets for one keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translate_x: f32,
    pub translate_y: f32,
    pub scale: f32,
    pub rotation_deg: f32,
    pub opacity: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    /// Identity transform: no translation, unit scale, no rotation, full opacity.
    pub const fn identity() -> Self {
        Self {
            translate_x: 0.0,
            translate_y: 0.0,
            scale: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.translate_x.is_finite()
            && self.translate_y.is_finite()
            && self.scale.is_finite()
            && self.rotation_deg.is_finite()
            && self.opacity.is_finite()
    }

    fn lerp(self, target: Self, t: f32) -> Self {
        Self {
            translate_x: self.translate_x + (target.translate_x - self.translate_x) * t,
            translate_y: self.translate_y + (target.translate_y - self.translate_y) * t,
            scale: self.scale + (target.scale - self.scale) * t,
            rotation_deg: self.rotation_deg + (target.rotation_deg - self.rotation_deg) * t,
            opacity: self.opacity + (target.opacity - self.opacity) * t,
        }
    }
}

/// One animation keyframe at a millisecond offset from the clip start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub at_ms: u64,
    pub transform: Transform,
    pub easing: Easing,
}

impl Keyframe {
    pub fn new(at_ms: u64, transform: Transform, easing: Easing) -> Self {
        Self {
            at_ms,
            transform,
            easing,
        }
    }
}

/// Playback behavior when the clip reaches its duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    Once,
    Loop,
}

/// A validated animation clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub schema: String,
    pub name: String,
    pub duration_ms: u64,
    pub playback: PlaybackMode,
    pub keyframes: Vec<Keyframe>,
}

impl AnimationClip {
    /// Construct a clip, sorting keyframes by time and rejecting invalid input.
    ///
    /// Keyframe times must be strictly increasing and must not exceed
    /// `duration_ms`. Every transform component must be finite.
    pub fn new(
        name: &str,
        duration_ms: u64,
        playback: PlaybackMode,
        mut keyframes: Vec<Keyframe>,
    ) -> Result<Self, EmbodimentError> {
        keyframes.sort_by_key(|keyframe| keyframe.at_ms);
        let clip = Self {
            schema: ANIMATION_SCHEMA.to_owned(),
            name: name.to_owned(),
            duration_ms,
            playback,
            keyframes,
        };
        clip.validate()?;
        Ok(clip)
    }

    pub fn validate(&self) -> Result<(), EmbodimentError> {
        if self.schema != ANIMATION_SCHEMA {
            return Err(EmbodimentError::CorruptData("animation schema"));
        }
        validate_animation_name(&self.name)?;
        if self.duration_ms == 0 {
            return Err(EmbodimentError::InvalidInput("animation duration"));
        }
        if self.keyframes.is_empty() || self.keyframes.len() > MAX_KEYFRAMES {
            return Err(EmbodimentError::InvalidInput("animation keyframe count"));
        }
        let mut previous: Option<u64> = None;
        for keyframe in &self.keyframes {
            if keyframe.at_ms > self.duration_ms {
                return Err(EmbodimentError::InvalidInput("animation keyframe time"));
            }
            if !keyframe.transform.is_finite() {
                return Err(EmbodimentError::InvalidInput("animation transform"));
            }
            if previous == Some(keyframe.at_ms) {
                return Err(EmbodimentError::InvalidInput("animation keyframe time"));
            }
            previous = Some(keyframe.at_ms);
        }
        Ok(())
    }

    /// Sample the transform at a local time, clamping outside the keyframe span.
    pub fn sample(&self, time_ms: u64) -> Transform {
        let Some(first) = self.keyframes.first() else {
            return Transform::identity();
        };
        let Some(last) = self.keyframes.last() else {
            return Transform::identity();
        };
        if time_ms <= first.at_ms {
            return first.transform;
        }
        if time_ms >= last.at_ms {
            return last.transform;
        }
        for pair in self.keyframes.windows(2) {
            let start = pair[0];
            let end = pair[1];
            if time_ms >= start.at_ms && time_ms <= end.at_ms {
                let span = (end.at_ms - start.at_ms) as f32;
                let progress = if span == 0.0 {
                    1.0
                } else {
                    (time_ms - start.at_ms) as f32 / span
                };
                return start
                    .transform
                    .lerp(end.transform, start.easing.apply(progress));
            }
        }
        last.transform
    }
}

/// Reported state of an [`AnimationPlayer`] after advancing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationState {
    pub clip_name: String,
    /// Clamped/looped local time within the clip.
    pub time_ms: u64,
    /// Monotonic elapsed time accumulated by the player.
    pub total_elapsed_ms: u64,
    /// True once a `Once` clip has reached its duration.
    pub finished: bool,
    pub transform: Transform,
}

/// Pure deterministic animation player.
#[derive(Debug, Clone)]
pub struct AnimationPlayer {
    clip: AnimationClip,
    total_elapsed_ms: u64,
}

impl AnimationPlayer {
    pub fn new(clip: AnimationClip) -> Self {
        Self {
            clip,
            total_elapsed_ms: 0,
        }
    }

    pub fn clip(&self) -> &AnimationClip {
        &self.clip
    }

    /// Reset accumulated time to the start of the clip.
    pub fn reset(&mut self) {
        self.total_elapsed_ms = 0;
    }

    /// Jump to an absolute elapsed time without advancing frame by frame.
    pub fn seek(&mut self, total_elapsed_ms: u64) -> AnimationState {
        self.total_elapsed_ms = total_elapsed_ms;
        self.state()
    }

    /// Advance by elapsed milliseconds and report the resulting state.
    ///
    /// `Once` clips clamp at the duration and set `finished`; `Loop` clips wrap
    /// back to the start and never report `finished`.
    pub fn advance(&mut self, elapsed_ms: u64) -> AnimationState {
        self.total_elapsed_ms = self.total_elapsed_ms.saturating_add(elapsed_ms);
        self.state()
    }

    /// Report the current state without advancing.
    pub fn state(&self) -> AnimationState {
        let (time_ms, finished) = match self.clip.playback {
            PlaybackMode::Once => {
                let finished = self.total_elapsed_ms >= self.clip.duration_ms;
                (self.total_elapsed_ms.min(self.clip.duration_ms), finished)
            }
            PlaybackMode::Loop => (self.total_elapsed_ms % self.clip.duration_ms, false),
        };
        AnimationState {
            clip_name: self.clip.name.clone(),
            time_ms,
            total_elapsed_ms: self.total_elapsed_ms,
            finished,
            transform: self.clip.sample(time_ms),
        }
    }
}

/// Broad motion category chosen for an avatar at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionMode {
    /// Quiet standing sway.
    Idle,
    /// Slow ambient drift.
    Ambient,
    /// Energetic movement.
    Active,
    /// Very slow resting breathing.
    Resting,
}

impl MotionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Ambient => "ambient",
            Self::Active => "active",
            Self::Resting => "resting",
        }
    }

    /// Deterministic clip name used by [`AvatarState::clip`].
    pub const fn clip_name(self) -> &'static str {
        match self {
            Self::Idle => "idle-sway",
            Self::Ambient => "ambient-drift",
            Self::Active => "active-motion",
            Self::Resting => "rest-breathe",
        }
    }
}

/// Context signal that actually contributed to an [`AvatarState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateSignal {
    Emotion,
    Activity,
    World,
    Weather,
}

impl StateSignal {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Emotion => "emotion",
            Self::Activity => "activity",
            Self::World => "world",
            Self::Weather => "weather",
        }
    }
}

/// Deterministic avatar state projected from optional emotion/activity/world.
///
/// Missing inputs never cause a fabricated signal: with no emotion the
/// expression stays [`Expression::Neutral`] and the gesture stays
/// [`Gesture::None`]; the `signals` list records exactly which contexts were
/// present. All dimensions are clamped so non-finite inputs degrade to zero
/// rather than panicking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarState {
    pub schema: String,
    /// Core UUID carried by the emotion snapshot, when one was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_uuid: Option<String>,
    pub expression: Expression,
    pub gesture: Gesture,
    pub motion: MotionMode,
    pub clip_name: String,
    /// Schematic activation in `0..=100`.
    pub energy: u8,
    /// Signals actually used, in stable order.
    pub signals: Vec<StateSignal>,
}

impl AvatarState {
    pub fn validate(&self) -> Result<(), EmbodimentError> {
        if self.schema != AVATAR_STATE_SCHEMA {
            return Err(EmbodimentError::CorruptData("avatar state schema"));
        }
        if self.energy > MAX_AVATAR_ENERGY {
            return Err(EmbodimentError::InvalidInput("avatar state energy"));
        }
        validate_animation_name(&self.clip_name)?;
        if let Some(core_uuid) = &self.core_uuid {
            if core_uuid.trim().is_empty()
                || core_uuid.chars().count() > 128
                || core_uuid.chars().any(char::is_control)
            {
                return Err(EmbodimentError::InvalidInput("avatar state core uuid"));
            }
        }
        if self.signals.len()
            > [
                StateSignal::Emotion,
                StateSignal::Activity,
                StateSignal::World,
                StateSignal::Weather,
            ]
            .len()
        {
            return Err(EmbodimentError::InvalidInput("avatar state signals"));
        }
        Ok(())
    }

    /// Build the deterministic looping clip named by this state.
    ///
    /// The clip is a schematic sway/bob (or breathing) loop. It carries no
    /// decoded art or rig data and is always safe to sample.
    pub fn clip(&self) -> Result<AnimationClip, EmbodimentError> {
        self.validate()?;
        let energy = f32::from(self.energy) / f32::from(MAX_AVATAR_ENERGY);
        let (duration_ms, sway, bob, scale_peak) = match self.motion {
            MotionMode::Idle => (2_000u64, 0.020f32, 0.010f32, 1.000f32),
            MotionMode::Ambient => (1_600u64, 0.050f32, 0.020f32, 1.005f32),
            MotionMode::Active => (1_000u64, 0.100f32, 0.035f32, 1.020f32),
            MotionMode::Resting => (4_000u64, 0.010f32, 0.015f32, 1.010f32),
        };
        let gain = 0.5 + 0.5 * energy;
        let sway = sway * gain;
        let bob = bob * gain;
        let breath = 1.0 + (scale_peak - 1.0) * gain;
        let halfway = duration_ms / 2;
        let keyframes = vec![
            Keyframe::new(
                0,
                Transform {
                    translate_x: -sway,
                    ..Transform::identity()
                },
                Easing::EaseInOut,
            ),
            Keyframe::new(
                halfway,
                Transform {
                    translate_x: sway,
                    translate_y: -bob,
                    scale: breath,
                    ..Transform::identity()
                },
                Easing::EaseInOut,
            ),
            Keyframe::new(
                duration_ms,
                Transform {
                    translate_x: -sway,
                    ..Transform::identity()
                },
                Easing::EaseInOut,
            ),
        ];
        AnimationClip::new(&self.clip_name, duration_ms, PlaybackMode::Loop, keyframes)
    }
}

/// Project optional emotion, activity, and world snapshots into an avatar state.
///
/// This is a pure, total function: identical inputs always produce an identical
/// state. Absent inputs are recorded as absent in `signals` and never replaced
/// with invented emotion or world data.
pub fn project_avatar_state(
    emotion: Option<&EmotionState>,
    activity: Option<&Activity>,
    world: Option<&WorldSnapshot>,
) -> AvatarState {
    let mut signals = Vec::new();
    let mut core_uuid = None;
    let mut expression = Expression::Neutral;
    let mut gesture = Gesture::None;
    let mut energy = 40.0f32;
    let mut force_active = false;
    let mut force_rest = false;

    if let Some(emotion) = emotion {
        signals.push(StateSignal::Emotion);
        if !emotion.core_uuid.trim().is_empty() {
            core_uuid = Some(emotion.core_uuid.clone());
        }
        let dimensions = emotion.dimensions;
        let valence = signed_unit(dimensions.valence);
        let arousal = unit(dimensions.arousal);
        let joy = unit(dimensions.joy);
        let calm = unit(dimensions.calm);
        let curiosity = unit(dimensions.curiosity);
        let tension = unit(dimensions.tension);

        expression = if tension >= 0.5 {
            Expression::Angry
        } else if joy >= 0.5 {
            Expression::Happy
        } else if valence <= -0.3 {
            Expression::Sad
        } else if arousal >= 0.75 {
            Expression::Surprised
        } else if curiosity >= 0.5 {
            Expression::Thinking
        } else {
            Expression::Neutral
        };

        gesture = if tension >= 0.5 {
            Gesture::Shrug
        } else if joy >= 0.5 {
            Gesture::ThumbsUp
        } else if arousal >= 0.7 {
            Gesture::Wave
        } else if curiosity >= 0.5 {
            Gesture::Point
        } else if calm >= 0.5 {
            Gesture::Nod
        } else {
            Gesture::None
        };

        energy += 25.0 * arousal + 15.0 * joy + 10.0 * curiosity - 15.0 * calm - 10.0 * tension;
    }

    if let Some(activity) = activity {
        signals.push(StateSignal::Activity);
        match activity.status {
            ActivityStatus::Active => {
                energy += 15.0;
                force_active = true;
            }
            ActivityStatus::Paused => energy -= 5.0,
            ActivityStatus::Completed | ActivityStatus::Cancelled => {
                energy -= 5.0;
                force_rest = true;
            }
            ActivityStatus::Planned => {}
        }
    }

    if let Some(world) = world {
        signals.push(StateSignal::World);
        match world.time_of_day {
            TimeOfDay::Night | TimeOfDay::Dusk => {
                energy -= 10.0;
                force_rest = true;
            }
            TimeOfDay::Evening => energy -= 5.0,
            TimeOfDay::Morning | TimeOfDay::Midday | TimeOfDay::Afternoon => energy += 5.0,
            TimeOfDay::Dawn => {}
        }
        match world.season {
            Season::Winter => energy -= 5.0,
            Season::Summer => energy += 5.0,
            Season::Spring | Season::Autumn | Season::Unknown => {}
        }
        if let Some(weather) = &world.weather {
            signals.push(StateSignal::Weather);
            let condition = weather.condition.to_lowercase();
            let stormy = [
                "rain", "snow", "storm", "thunder", "drizzle", "sleet", "hail", "fog", "shower",
            ]
            .iter()
            .any(|term| condition.contains(term));
            if stormy {
                energy -= 15.0;
                force_rest = true;
            } else if ["clear", "sun", "bright"]
                .iter()
                .any(|term| condition.contains(term))
            {
                energy += 5.0;
            }
            if let Some(temperature) = weather.feels_like_celsius.or(weather.temperature_celsius) {
                if temperature.is_finite() && !(5.0..28.0).contains(&temperature) {
                    energy -= 5.0;
                }
            }
        }
    }

    let energy_value = if energy.is_finite() {
        energy.round().clamp(0.0, f32::from(MAX_AVATAR_ENERGY))
    } else {
        40.0
    };
    let energy = energy_value as u8;

    let motion = if force_active {
        MotionMode::Active
    } else if force_rest || energy <= 25 {
        MotionMode::Resting
    } else if energy >= 65 {
        MotionMode::Active
    } else if energy >= 45 {
        MotionMode::Ambient
    } else {
        MotionMode::Idle
    };

    AvatarState {
        schema: AVATAR_STATE_SCHEMA.to_owned(),
        core_uuid,
        expression,
        gesture,
        motion,
        clip_name: motion.clip_name().to_owned(),
        energy,
        signals,
    }
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn signed_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}
