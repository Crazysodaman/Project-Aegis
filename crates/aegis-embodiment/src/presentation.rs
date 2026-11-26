//! Deterministic, additive presentation decisions derived from world, emotion,
//! and activity context.
//!
//! Nothing in this module writes to a Core's appearance or asset library. The
//! outfit selector reads live, Core-owned wardrobe assets and ranks them from
//! their tags and name only. Tags are the only structured clothing metadata in
//! the store today, so the environmental safety checks and preference scoring
//! here are an explicit, documented heuristic rather than a semantic wardrobe
//! model.
//!
//! The render path reuses the existing headless CPU rasterizer; it does not add
//! a native window, GPU renderer, image decoder, or UI.

use aegis_contracts::personal::{
    Activity, ActivityStatus, EmotionDimensions, EmotionState, OutfitDecisionRecord,
    MAX_ASSET_ID_CHARS, MAX_ID_CHARS, MAX_OUTFIT_ASSET_IDS, MAX_OUTFIT_RATIONALE_CHARS,
    MAX_TIMESTAMP_CHARS,
};
use aegis_contracts::world::{Season, TimeOfDay, WorldSnapshot};

use crate::appearance::Appearance;
use crate::asset::{Asset, AssetKind};
use crate::error::EmbodimentError;
use crate::raster::render_bmp;
use crate::scene::AvatarScene;
use crate::store::EmbodimentStore;

/// Schema identifier for an outfit decision.
pub const OUTFIT_DECISION_SCHEMA: &str = "aegis.embodiment.outfit.v1";
/// Schema identifier for a rendered presentation frame descriptor.
pub const PRESENTATION_FRAME_SCHEMA: &str = "aegis.embodiment.presentation.v1";
/// Maximum number of wardrobe assets selected by one decision.
pub const MAX_OUTFIT_SELECTION: usize = 8;
/// Lowest voice rate the projection may emit, in percent of normal.
pub const MIN_VOICE_RATE_PERCENT: u16 = 60;
/// Highest voice rate the projection may emit, in percent of normal.
pub const MAX_VOICE_RATE_PERCENT: u16 = 150;
/// Lowest voice volume the projection may emit, in percent of normal.
pub const MIN_VOICE_VOLUME_PERCENT: u16 = 40;
/// Highest voice volume the projection may emit, in percent of normal.
pub const MAX_VOICE_VOLUME_PERCENT: u16 = 100;
/// Default presentation frame width in pixels.
pub const PRESENTATION_FRAME_WIDTH: u32 = 256;
/// Default presentation frame height in pixels.
pub const PRESENTATION_FRAME_HEIGHT: u32 = 256;

/// Below this feels-like temperature cold safety applies, in Celsius.
const COLD_THRESHOLD_CELSIUS: f64 = 5.0;
/// At or above this feels-like temperature heat safety applies, in Celsius.
const HOT_THRESHOLD_CELSIUS: f64 = 28.0;
/// At or above this humidity, precipitation safety applies.
const HIGH_HUMIDITY_PERCENT: u8 = 90;

const WARM_TERMS: &[&str] = &[
    "warm",
    "coat",
    "jacket",
    "sweater",
    "thermal",
    "insulated",
    "winter",
    "fleece",
    "parka",
    "hoodie",
];
const COOL_TERMS: &[&str] = &[
    "cool",
    "light",
    "breathable",
    "summer",
    "linen",
    "shorts",
    "sleeveless",
    "tank",
    "sandals",
];
const WET_TERMS: &[&str] = &[
    "waterproof",
    "weatherproof",
    "rain",
    "boots",
    "coat",
    "jacket",
    "umbrella",
    "poncho",
    "snow",
];
const COZY_TERMS: &[&str] = &["cozy", "soft", "comfortable", "soothing"];
const BRIGHT_TERMS: &[&str] = &["bright", "colorful", "festive", "cheerful", "happy"];

/// Borrowed context used to make a deterministic presentation decision.
///
/// Every field is optional. A missing world, emotion, or activity never causes
/// a fabricated value; it simply removes that signal from the score.
///
/// `required_tags` is a hard identity constraint: every tag listed here must be
/// present on an asset or the asset is rejected before scoring. Callers use it
/// for identity/protective requirements such as `armor` or an approved uniform;
/// it always outranks preference and comfort signals.
#[derive(Debug, Clone, Copy, Default)]
pub struct PresentationContext<'a> {
    pub world: Option<&'a WorldSnapshot>,
    pub emotion: Option<&'a EmotionState>,
    pub activity: Option<&'a Activity>,
    pub personality_tags: &'a [String],
    pub preference_tags: &'a [String],
    pub required_tags: &'a [String],
}

impl<'a> PresentationContext<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_world(mut self, world: &'a WorldSnapshot) -> Self {
        self.world = Some(world);
        self
    }

    pub fn with_emotion(mut self, emotion: &'a EmotionState) -> Self {
        self.emotion = Some(emotion);
        self
    }

    pub fn with_activity(mut self, activity: &'a Activity) -> Self {
        self.activity = Some(activity);
        self
    }

    pub fn with_personality_tags(mut self, tags: &'a [String]) -> Self {
        self.personality_tags = tags;
        self
    }

    pub fn with_preference_tags(mut self, tags: &'a [String]) -> Self {
        self.preference_tags = tags;
        self
    }

    /// Require every listed tag (case-insensitive) as a hard identity constraint.
    ///
    /// Assets missing any required tag are rejected before scoring. This is the
    /// supported way to require protective or identity gear such as `armor`.
    pub fn with_required_tags(mut self, tags: &'a [String]) -> Self {
        self.required_tags = tags;
        self
    }
}

/// One wardrobe asset that was excluded from a decision.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutfitRejection {
    pub asset_id: String,
    pub reason: String,
}

/// Deterministic outfit decision for one Core.
///
/// `available` is false only when no live, Core-owned wardrobe asset satisfied
/// the hard constraints (including an empty wardrobe). An unavailable decision
/// always has an empty `asset_ids`, so no clothing is ever invented.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutfitDecision {
    pub schema: String,
    pub core_uuid: String,
    pub available: bool,
    pub asset_ids: Vec<String>,
    pub rationale: String,
    pub constraints: Vec<String>,
    pub rejected: Vec<OutfitRejection>,
    pub considered: usize,
}

impl OutfitDecision {
    /// Convert this decision into the shared, append-only personal contract.
    ///
    /// The conversion is additive and total: it copies the selected live asset
    /// ids and the rationale into one [`OutfitDecisionRecord`]. Embodiment has no
    /// dependency on `aegis-core-sdk`, so the caller supplies the record `id`
    /// and `decided_at` timestamp and is solely responsible for persistence.
    ///
    /// Every field is truncated to its contract bound, so the returned record
    /// always satisfies the shared length limits (an empty caller-supplied `id`
    /// or timestamp remains the caller's validation error).
    pub fn to_personal_record(
        &self,
        id: impl Into<String>,
        decided_at: impl Into<String>,
    ) -> OutfitDecisionRecord {
        OutfitDecisionRecord {
            id: truncate_chars(&id.into(), MAX_ID_CHARS),
            decided_at: truncate_chars(&decided_at.into(), MAX_TIMESTAMP_CHARS),
            asset_ids: self
                .asset_ids
                .iter()
                .take(MAX_OUTFIT_ASSET_IDS)
                .map(|asset_id| truncate_chars(asset_id, MAX_ASSET_ID_CHARS))
                .collect(),
            rationale: truncate_chars(&self.rationale, MAX_OUTFIT_RATIONALE_CHARS),
        }
    }
}

/// Provider-neutral speech style derived from an emotion state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VoiceProjection {
    pub rate_percent: u16,
    pub volume_percent: u16,
    pub style_label: String,
}

/// A composed scene plus the optional BMP produced by the existing rasterizer.
///
/// When the appearance has no renderable layers, `bmp` is `None` and the frame
/// remains a deterministic scene/expression descriptor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PresentationFrame {
    pub schema: String,
    pub core_uuid: String,
    pub expression: Option<String>,
    pub scene: AvatarScene,
    pub bmp: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub season: Option<Season>,
    pub time_of_day: Option<TimeOfDay>,
}

#[derive(Debug, Clone, Copy)]
struct WeatherProfile {
    cold: bool,
    hot: bool,
    wet: bool,
}

#[derive(Debug, Clone, Copy)]
struct ScoringInputs<'a> {
    weather: Option<WeatherProfile>,
    world: Option<&'a WorldSnapshot>,
    activity: Option<&'a Activity>,
    emotion: Option<&'a EmotionState>,
    personality_tags: &'a [String],
    preference_tags: &'a [String],
}

/// Select wardrobe assets for one Core from live inventory and context.
///
/// Hard constraints are applied before any preference scoring:
/// * the store and caller Core UUID must match;
/// * assets must be Core-owned, live, and `Wardrobe` kind;
/// * extreme cold, heat, or precipitation requires a matching safety tag;
/// * every caller-supplied required tag (for example `armor`) must be present.
///
/// The remaining score is a documented heuristic over asset tags/name. Ties are
/// broken by name then asset id, so equal inputs always produce equal output and
/// personality/preferences can never override a hard safety or identity gate.
pub fn select_outfit(
    store: &EmbodimentStore,
    core_uuid: &str,
    ctx: &PresentationContext<'_>,
    limit: usize,
) -> Result<OutfitDecision, EmbodimentError> {
    if store.core_uuid() != core_uuid {
        return Err(EmbodimentError::OwnershipMismatch);
    }

    let weather = weather_profile(ctx.world);
    let inputs = ScoringInputs {
        weather,
        world: ctx.world,
        activity: ctx.activity,
        emotion: ctx.emotion,
        personality_tags: ctx.personality_tags,
        preference_tags: ctx.preference_tags,
    };

    let assets = store.list_assets(None, true)?;
    let mut rejections = Vec::new();
    let mut considered = 0usize;
    let mut eligible: Vec<(i32, &Asset)> = Vec::new();

    for asset in &assets {
        if asset.kind != AssetKind::Wardrobe {
            continue;
        }
        considered += 1;
        if asset.core_uuid != core_uuid {
            rejections.push(OutfitRejection {
                asset_id: asset.asset_id.clone(),
                reason: "foreign-core".to_owned(),
            });
            continue;
        }
        if asset.deleted {
            rejections.push(OutfitRejection {
                asset_id: asset.asset_id.clone(),
                reason: "deleted".to_owned(),
            });
            continue;
        }
        let text = normalized(asset);
        if let Some(reason) = safety_rejection(weather, &text) {
            rejections.push(OutfitRejection {
                asset_id: asset.asset_id.clone(),
                reason: reason.to_owned(),
            });
            continue;
        }
        if let Some(missing) = missing_required_tag(asset, ctx.required_tags) {
            rejections.push(OutfitRejection {
                asset_id: asset.asset_id.clone(),
                reason: format!("required-tag:{missing}"),
            });
            continue;
        }
        eligible.push((score_asset(asset, &inputs), asset));
    }

    eligible.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.name.to_lowercase().cmp(&right.1.name.to_lowercase()))
            .then_with(|| left.1.asset_id.cmp(&right.1.asset_id))
    });

    let effective_limit = limit.min(MAX_OUTFIT_SELECTION);
    let selected: Vec<&Asset> = eligible
        .iter()
        .take(effective_limit)
        .map(|(_, asset)| *asset)
        .collect();
    let available = !eligible.is_empty();

    let mut constraints = vec![
        "core-ownership".to_owned(),
        "live-only".to_owned(),
        "wardrobe-kind".to_owned(),
    ];
    if let Some(profile) = weather {
        if profile.cold {
            constraints.push("cold-safety".to_owned());
        }
        if profile.hot {
            constraints.push("heat-safety".to_owned());
        }
        if profile.wet {
            constraints.push("precipitation-safety".to_owned());
        }
    }
    let required = normalized_required(ctx.required_tags);
    if !required.is_empty() {
        constraints.push("required-tags".to_owned());
        if required.iter().any(|tag| tag == "armor") {
            constraints.push("armor-identity".to_owned());
        }
    }

    let rationale = bounded_rationale(rationale(
        ctx,
        &constraints,
        considered,
        selected.len(),
        available,
        rejections.len(),
    ));

    Ok(OutfitDecision {
        schema: OUTFIT_DECISION_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        available,
        asset_ids: selected
            .iter()
            .map(|asset| asset.asset_id.clone())
            .collect(),
        rationale,
        constraints,
        rejected: rejections,
        considered,
    })
}

/// Project a bounded provider-neutral speech style from an emotion state.
///
/// `None` (no personal state) yields the neutral projection: 100% rate, 100%
/// volume, and the label `neutral`. All arithmetic is deterministic and clamped
/// to the declared bounds.
pub fn voice_projection(emotion: Option<&EmotionState>) -> VoiceProjection {
    let dimensions = emotion.map_or_else(EmotionDimensions::neutral, |state| state.dimensions);
    let valence = signed_unit(dimensions.valence);
    let arousal = unit(dimensions.arousal);
    let joy = unit(dimensions.joy);
    let calm = unit(dimensions.calm);
    let curiosity = unit(dimensions.curiosity);
    let tension = unit(dimensions.tension);

    let rate = 100.0 + (arousal * 25.0 + joy * 8.0 + tension * 8.0 - calm * 15.0).round();
    let volume = 100.0
        + (joy * 6.0 + valence.max(0.0) * 4.0 - tension * 6.0 - (-valence).max(0.0) * 4.0).round();

    let style_label = if tension >= 0.5 && arousal >= 0.4 {
        "tense"
    } else if joy >= 0.5 {
        "bright"
    } else if calm >= 0.5 {
        "calm"
    } else if curiosity >= 0.5 {
        "curious"
    } else if valence <= -0.3 {
        "subdued"
    } else if arousal >= 0.6 {
        "energetic"
    } else {
        "neutral"
    };

    VoiceProjection {
        rate_percent: clamp_percent(rate, MIN_VOICE_RATE_PERCENT, MAX_VOICE_RATE_PERCENT),
        volume_percent: clamp_percent(volume, MIN_VOICE_VOLUME_PERCENT, MAX_VOICE_VOLUME_PERCENT),
        style_label: style_label.to_owned(),
    }
}

/// Pick an existing named expression from the Core's appearance for `emotion`.
///
/// Candidate names are tried in deterministic priority order and matched
/// case-insensitively against names that already exist in
/// [`Appearance::expressions`]. The stored name is returned unchanged. If no
/// candidate exists, `None` is returned; an expression name is never invented.
pub fn expression_for_emotion(
    appearance: &Appearance,
    emotion: Option<&EmotionState>,
) -> Option<String> {
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(emotion) = emotion {
        let d = emotion.dimensions;
        if unit(d.joy) >= 0.5 {
            candidates.extend_from_slice(&["happy", "joy", "joyful", "smile", "cheerful"]);
        }
        if unit(d.tension) >= 0.5 {
            candidates.extend_from_slice(&[
                "angry",
                "tense",
                "stressed",
                "frustrated",
                "concerned",
            ]);
        }
        if signed_unit(d.valence) <= -0.3 {
            candidates.extend_from_slice(&["sad", "down", "upset", "sorrow"]);
        }
        if unit(d.arousal) >= 0.75 {
            candidates.extend_from_slice(&["surprised", "surprise", "excited"]);
        }
        if unit(d.curiosity) >= 0.5 {
            candidates.extend_from_slice(&["thinking", "curious", "interested"]);
        }
        if unit(d.calm) >= 0.5 {
            candidates.extend_from_slice(&["calm", "relaxed", "serene"]);
        }
    }
    candidates.extend_from_slice(&["neutral", "calm", "default", "rest"]);

    for candidate in candidates {
        if let Some(name) = appearance
            .expressions
            .keys()
            .find(|name| name.eq_ignore_ascii_case(candidate))
        {
            return Some(name.clone());
        }
    }
    None
}

/// Compose the Core's persisted appearance and render it with the existing BMP
/// rasterizer.
///
/// This is read-only: it never mutates appearance, assets, or wardrobe. When
/// there are no renderable layers it returns a deterministic scene descriptor
/// with `bmp = None` instead of failing or inventing art. The `world` value is
/// recorded in the frame metadata only; it does not modify approved appearance.
pub fn render_presentation_frame(
    store: &EmbodimentStore,
    core_uuid: &str,
    world: Option<&WorldSnapshot>,
    emotion: Option<&EmotionState>,
) -> Result<PresentationFrame, EmbodimentError> {
    if store.core_uuid() != core_uuid {
        return Err(EmbodimentError::OwnershipMismatch);
    }

    let appearance = store.appearance()?;
    let assets = store.list_assets(None, false)?;
    let expression = expression_for_emotion(&appearance, emotion);
    let scene = AvatarScene::compose(&appearance, &assets, expression.as_deref(), None)?;
    let bmp = if scene.layers.is_empty() {
        None
    } else {
        Some(render_bmp(
            &scene,
            PRESENTATION_FRAME_WIDTH,
            PRESENTATION_FRAME_HEIGHT,
        )?)
    };

    Ok(PresentationFrame {
        schema: PRESENTATION_FRAME_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        expression,
        scene,
        bmp,
        width: PRESENTATION_FRAME_WIDTH,
        height: PRESENTATION_FRAME_HEIGHT,
        season: world.map(|snapshot| snapshot.season),
        time_of_day: world.map(|snapshot| snapshot.time_of_day),
    })
}

fn weather_profile(world: Option<&WorldSnapshot>) -> Option<WeatherProfile> {
    let world = world?;
    let weather = world.weather.as_ref()?;
    let temperature = weather.feels_like_celsius.or(weather.temperature_celsius);
    let condition = weather.condition.to_lowercase();
    let stormy_condition = [
        "rain", "snow", "storm", "drizzle", "sleet", "hail", "shower", "thunder", "fog",
    ]
    .iter()
    .any(|term| condition.contains(term));
    let humid = weather
        .humidity_percent
        .is_some_and(|percent| percent >= HIGH_HUMIDITY_PERCENT);
    Some(WeatherProfile {
        cold: temperature.is_some_and(|value| value <= COLD_THRESHOLD_CELSIUS),
        hot: temperature.is_some_and(|value| value >= HOT_THRESHOLD_CELSIUS),
        wet: stormy_condition || humid,
    })
}

fn safety_rejection(weather: Option<WeatherProfile>, text: &str) -> Option<&'static str> {
    let weather = weather?;
    if weather.cold && !contains_any(text, WARM_TERMS) {
        return Some("cold-safety");
    }
    if weather.hot && !contains_any(text, COOL_TERMS) {
        return Some("heat-safety");
    }
    if weather.wet && !contains_any(text, WET_TERMS) {
        return Some("precipitation-safety");
    }
    None
}

fn score_asset(asset: &Asset, inputs: &ScoringInputs<'_>) -> i32 {
    let text = normalized(asset);
    let mut score = 0i32;

    if let Some(weather) = inputs.weather {
        if weather.cold && contains_any(&text, WARM_TERMS) {
            score += 40;
        }
        if weather.hot && contains_any(&text, COOL_TERMS) {
            score += 40;
        }
        if weather.wet && contains_any(&text, WET_TERMS) {
            score += 30;
        }
    }

    if let Some(world) = inputs.world {
        if text.contains(world.season.as_str()) {
            score += 20;
        }
        if text.contains(world.time_of_day.as_str()) {
            score += 15;
        } else {
            match world.time_of_day {
                TimeOfDay::Night | TimeOfDay::Evening | TimeOfDay::Dusk => {
                    if contains_any(&text, &["night", "evening", "dark", "formal", "sleep"]) {
                        score += 8;
                    }
                }
                _ => {
                    if contains_any(&text, &["day", "casual", "light", "sun"]) {
                        score += 8;
                    }
                }
            }
        }
    }

    if let Some(activity) = inputs.activity {
        let mut activity_score = 0i32;
        for token in activity
            .title
            .to_lowercase()
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| token.len() >= 3)
        {
            if text.contains(token) {
                activity_score = (activity_score + 8).min(24);
            }
        }
        if activity.status == ActivityStatus::Active
            && contains_any(
                &text,
                &["active", "sport", "workout", "casual", "comfortable"],
            )
        {
            activity_score += 6;
        }
        score += activity_score;
    }

    if let Some(emotion) = inputs.emotion {
        let d = emotion.dimensions;
        if unit(d.joy) >= 0.5 && contains_any(&text, BRIGHT_TERMS) {
            score += 10;
        }
        if unit(d.calm) >= 0.5 && contains_any(&text, COZY_TERMS) {
            score += 10;
        }
        if unit(d.tension) >= 0.5 && contains_any(&text, COZY_TERMS) {
            score += 12;
        }
        if unit(d.arousal) >= 0.6 && contains_any(&text, &["active", "sport", "dynamic", "bright"])
        {
            score += 8;
        }
        if unit(d.curiosity) >= 0.5
            && contains_any(&text, &["adventurous", "practical", "casual", "utility"])
        {
            score += 8;
        }
        if signed_unit(d.valence) <= -0.3 && contains_any(&text, &["cozy", "soft", "warm", "muted"])
        {
            score += 8;
        }
    }

    let personality_hits = tag_overlap(asset, inputs.personality_tags);
    score += (personality_hits * 12).min(36);
    let preference_hits = tag_overlap(asset, inputs.preference_tags);
    score += (preference_hits * 15).min(45);

    score
}

fn rationale(
    ctx: &PresentationContext<'_>,
    constraints: &[String],
    considered: usize,
    selected: usize,
    available: bool,
    rejected: usize,
) -> String {
    if !available {
        if considered == 0 {
            return "no live wardrobe assets are available for this Core".to_owned();
        }
        return format!(
            "no live wardrobe asset satisfied hard constraints [{}]; {rejected} of {considered} rejected",
            constraints.join(",")
        );
    }

    let mut factors = Vec::new();
    if let Some(world) = ctx.world {
        factors.push(world.season.as_str());
        factors.push(world.time_of_day.as_str());
        if world.weather.is_some() {
            factors.push("weather");
        }
    }
    if ctx.activity.is_some() {
        factors.push("activity");
    }
    if ctx.emotion.is_some() {
        factors.push("emotion");
    }
    if !ctx.personality_tags.is_empty() {
        factors.push("personality");
    }
    if !ctx.preference_tags.is_empty() {
        factors.push("preferences");
    }
    if !ctx.required_tags.is_empty() {
        factors.push("required-tags");
    }
    let factors = if factors.is_empty() {
        "default".to_owned()
    } else {
        factors.join("+")
    };
    format!(
        "selected {selected} of {considered} eligible wardrobe assets using {factors}; constraints [{}]; {rejected} rejected",
        constraints.join(",")
    )
}

fn bounded_rationale(value: String) -> String {
    truncate_chars(&value, MAX_OUTFIT_RATIONALE_CHARS)
}

fn normalized_required(required: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in required {
        let tag = tag.trim().to_lowercase();
        if !tag.is_empty() && !normalized.contains(&tag) {
            normalized.push(tag);
        }
    }
    normalized
}

fn missing_required_tag(asset: &Asset, required: &[String]) -> Option<String> {
    for want in required {
        let want = want.trim();
        if want.is_empty() {
            continue;
        }
        let present = asset
            .tags
            .iter()
            .any(|tag| tag.trim().eq_ignore_ascii_case(want));
        if !present {
            return Some(truncate_chars(&want.to_lowercase(), 64));
        }
    }
    None
}

fn tag_overlap(asset: &Asset, wanted: &[String]) -> i32 {
    let mut hits = 0i32;
    for want in wanted {
        let want = want.trim().to_lowercase();
        if want.is_empty() {
            continue;
        }
        if asset.tags.iter().any(|tag| tag.to_lowercase() == want) {
            hits += 1;
        }
    }
    hits
}

fn normalized(asset: &Asset) -> String {
    let mut text = asset.name.to_lowercase();
    for tag in &asset.tags {
        text.push(' ');
        text.push_str(&tag.to_lowercase());
    }
    text
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(*term))
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
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

fn clamp_percent(value: f32, minimum: u16, maximum: u16) -> u16 {
    if !value.is_finite() {
        return maximum;
    }
    let rounded = value.round();
    if rounded < f32::from(minimum) {
        minimum
    } else if rounded > f32::from(maximum) {
        maximum
    } else {
        rounded as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_voice_projection_is_unchanged() {
        let projection = voice_projection(None);
        assert_eq!(projection.rate_percent, 100);
        assert_eq!(projection.volume_percent, 100);
        assert_eq!(projection.style_label, "neutral");
    }

    #[test]
    fn term_matching_is_case_insensitive() {
        let text = "winter coat waterproof";
        assert!(contains_any(text, WARM_TERMS));
        assert!(contains_any(text, WET_TERMS));
        assert!(!contains_any(text, COOL_TERMS));
    }
}
