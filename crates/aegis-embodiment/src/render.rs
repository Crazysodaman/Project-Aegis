//! Renderer abstractions and a deterministic headless scene renderer.
//!
//! [`SceneRenderer`] is the contract a real GPU/raster renderer would implement.
//! [`HeadlessSceneRenderer`] is the only implementation shipped here: it emits a
//! deterministic, serializable [`SceneDescription`] describing layers, theme,
//! active motion, and animation state. It does **not** rasterize pixels, touch a
//! GPU, open a window, or perform lip-sync; the description explicitly reports
//! those capabilities as disabled.

use serde::{Deserialize, Serialize};

use crate::animation::AnimationState;
use crate::appearance::Theme;
use crate::error::EmbodimentError;
use crate::scene::{ActiveMotion, AvatarScene, SceneLayer};

/// Schema identifier for a headless scene description.
pub const SCENE_DESCRIPTION_SCHEMA: &str = "aegis.embodiment.scene_description.v1";
/// Identifier reported by [`HeadlessSceneRenderer`].
pub const HEADLESS_RENDERER: &str = "headless-scene";

/// Deterministic, serializable description of a scene at one instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneDescription {
    pub schema: String,
    /// Schema of the source [`AvatarScene`] this description was rendered from.
    pub scene_schema: String,
    pub renderer: String,
    pub core_uuid: String,
    pub theme: Theme,
    pub layers: Vec<SceneLayer>,
    pub expression: Option<ActiveMotion>,
    pub gesture: Option<ActiveMotion>,
    pub animation: Option<AnimationState>,
    /// Always false: no pixels are produced by this renderer.
    pub rasterized: bool,
    /// Always false: no GPU work is performed by this renderer.
    pub gpu_used: bool,
    /// Always false: no native window is opened by this renderer.
    pub windowed: bool,
}

impl SceneDescription {
    /// Serialize to compact JSON. Identical inputs produce identical output.
    pub fn to_json(&self) -> Result<String, EmbodimentError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Serialize to pretty JSON. Identical inputs produce identical output.
    pub fn to_json_pretty(&self) -> Result<String, EmbodimentError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// Contract implemented by any avatar scene renderer.
pub trait SceneRenderer: std::fmt::Debug + Send + Sync {
    /// Human-readable renderer name, recorded in the description.
    fn name(&self) -> &str;

    /// Produce a scene description for `scene` at the given animation state.
    fn render(
        &self,
        scene: &AvatarScene,
        animation: Option<&AnimationState>,
    ) -> Result<SceneDescription, EmbodimentError>;
}

/// Marker sub-trait for renderers that target the avatar pipeline specifically.
pub trait AvatarRenderer: SceneRenderer {}

/// Renderer that emits a deterministic scene description without drawing.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeadlessSceneRenderer;

impl HeadlessSceneRenderer {
    pub const fn new() -> Self {
        Self
    }
}

impl SceneRenderer for HeadlessSceneRenderer {
    fn name(&self) -> &str {
        HEADLESS_RENDERER
    }

    fn render(
        &self,
        scene: &AvatarScene,
        animation: Option<&AnimationState>,
    ) -> Result<SceneDescription, EmbodimentError> {
        scene.validate()?;
        let mut layers = scene.layers.clone();
        layers.sort_by(|left, right| {
            left.z_index
                .cmp(&right.z_index)
                .then(left.role.cmp(&right.role))
                .then(left.asset_id.cmp(&right.asset_id))
        });
        Ok(SceneDescription {
            schema: SCENE_DESCRIPTION_SCHEMA.to_owned(),
            scene_schema: scene.schema.clone(),
            renderer: self.name().to_owned(),
            core_uuid: scene.core_uuid.clone(),
            theme: scene.theme.clone(),
            layers,
            expression: scene.expression.clone(),
            gesture: scene.gesture.clone(),
            animation: animation.cloned(),
            rasterized: false,
            gpu_used: false,
            windowed: false,
        })
    }
}

impl AvatarRenderer for HeadlessSceneRenderer {}
