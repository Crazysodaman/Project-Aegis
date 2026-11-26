//! Renderer-agnostic avatar scene model.
//!
//! A scene describes *what* should be drawn (layer roles, asset references,
//! theme colors, and the active expression/gesture) without prescribing *how*
//! it is drawn. Composing a scene from an [`Appearance`] plus asset metadata
//! validates that every referenced asset exists in the owning Core, is live,
//! and has the expected [`AssetKind`]. No pixels, GPU commands, or native
//! window are produced here; see [`crate::render`] for the headless scene path.
//!
//! Hair, eye, and overlay layers are part of the model so a renderer can bind
//! them explicitly. Only the layers that exist in the persisted appearance
//! (base avatar and wardrobe) are auto-composed today; no appearance fields are
//! invented or inferred from asset tags.

use serde::{Deserialize, Serialize};

use crate::appearance::{validate_animation_name, validate_theme, Appearance, Theme};
use crate::asset::{Asset, AssetKind};
use crate::error::EmbodimentError;

/// Schema identifier for a composed avatar scene.
pub const SCENE_SCHEMA: &str = "aegis.embodiment.scene.v1";
/// Maximum number of layers in one scene.
pub const MAX_SCENE_LAYERS: usize = 64;
/// Maximum characters in a scene asset reference.
pub const MAX_SCENE_ASSET_CHARS: usize = 128;

/// Semantic role of an avatar layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    /// Base body/avatar image.
    Base,
    /// Hair layer.
    Hair,
    /// Eye layer.
    Eyes,
    /// Outfit or wardrobe layer.
    Outfit,
    /// Free-form overlay layer (effects, accessories, UI frames).
    Overlay,
}

impl LayerRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Hair => "hair",
            Self::Eyes => "eyes",
            Self::Outfit => "outfit",
            Self::Overlay => "overlay",
        }
    }

    /// Conventional draw order used when composing from appearance state.
    pub const fn default_z(self) -> i32 {
        match self {
            Self::Base => 0,
            Self::Outfit => 10,
            Self::Hair => 20,
            Self::Eyes => 30,
            Self::Overlay => 100,
        }
    }
}

/// One validated layer reference inside an [`AvatarScene`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneLayer {
    pub role: LayerRole,
    pub asset_id: String,
    pub asset_kind: AssetKind,
    pub media_type: String,
    pub sha256: String,
    pub visible: bool,
    pub z_index: i32,
}

impl SceneLayer {
    /// Build a visible layer from a live asset record.
    pub fn from_asset(role: LayerRole, asset: &Asset, z_index: i32) -> Self {
        Self {
            role,
            asset_id: asset.asset_id.clone(),
            asset_kind: asset.kind,
            media_type: asset.media_type.clone(),
            sha256: asset.sha256.clone(),
            visible: true,
            z_index,
        }
    }

    pub fn validate(&self) -> Result<(), EmbodimentError> {
        validate_reference(&self.asset_id)?;
        if self.media_type.trim().is_empty()
            || self.media_type.chars().count() > MAX_SCENE_ASSET_CHARS
            || self.media_type.chars().any(char::is_control)
        {
            return Err(EmbodimentError::InvalidInput("scene layer media type"));
        }
        if self.sha256.trim().is_empty()
            || self.sha256.chars().count() > MAX_SCENE_ASSET_CHARS
            || self.sha256.chars().any(char::is_control)
        {
            return Err(EmbodimentError::InvalidInput("scene layer digest"));
        }
        Ok(())
    }
}

/// A named expression or gesture resolved to a Core-owned asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveMotion {
    pub name: String,
    pub asset_id: String,
}

impl ActiveMotion {
    pub fn new(name: &str, asset_id: &str) -> Result<Self, EmbodimentError> {
        validate_animation_name(name)?;
        validate_reference(asset_id)?;
        Ok(Self {
            name: name.trim().to_owned(),
            asset_id: asset_id.to_owned(),
        })
    }

    fn validate(&self) -> Result<(), EmbodimentError> {
        validate_animation_name(&self.name)?;
        validate_reference(&self.asset_id)
    }
}

/// A renderer-agnostic scene composed for exactly one Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvatarScene {
    pub schema: String,
    pub core_uuid: String,
    pub theme: Theme,
    pub layers: Vec<SceneLayer>,
    pub expression: Option<ActiveMotion>,
    pub gesture: Option<ActiveMotion>,
}

impl AvatarScene {
    /// Construct a scene from validated parts.
    ///
    /// Layers are sorted deterministically by `z_index`, role, then asset id.
    /// At most one `base`, `hair`, `eyes`, and `overlay` layer is allowed;
    /// `outfit` layers may repeat (for example, stacked wardrobe pieces).
    pub fn new(
        core_uuid: &str,
        theme: Theme,
        mut layers: Vec<SceneLayer>,
        expression: Option<ActiveMotion>,
        gesture: Option<ActiveMotion>,
    ) -> Result<Self, EmbodimentError> {
        layers.sort_by(|left, right| {
            left.z_index
                .cmp(&right.z_index)
                .then(left.role.cmp(&right.role))
                .then(left.asset_id.cmp(&right.asset_id))
        });
        let scene = Self {
            schema: SCENE_SCHEMA.to_owned(),
            core_uuid: core_uuid.to_owned(),
            theme,
            layers,
            expression,
            gesture,
        };
        scene.validate()?;
        Ok(scene)
    }

    /// Compose a scene from a persisted appearance and its resolved assets.
    ///
    /// Every referenced asset must be present in `assets`, owned by
    /// `appearance.core_uuid`, live (not soft-deleted), and of the expected
    /// kind. A foreign-Core reference returns [`EmbodimentError::OwnershipMismatch`];
    /// a kind mismatch returns [`EmbodimentError::WrongAssetKind`].
    pub fn compose(
        appearance: &Appearance,
        assets: &[Asset],
        expression: Option<&str>,
        gesture: Option<&str>,
    ) -> Result<Self, EmbodimentError> {
        appearance.validate(&appearance.core_uuid)?;

        let mut layers = Vec::new();
        if let Some(avatar_id) = &appearance.avatar_asset_id {
            let asset = require_asset(assets, &appearance.core_uuid, avatar_id, AssetKind::Avatar)?;
            layers.push(SceneLayer::from_asset(
                LayerRole::Base,
                asset,
                LayerRole::Base.default_z(),
            ));
        }
        for (index, wardrobe_id) in appearance.wardrobe.iter().enumerate() {
            let asset = require_asset(
                assets,
                &appearance.core_uuid,
                wardrobe_id,
                AssetKind::Wardrobe,
            )?;
            let offset = i32::try_from(index)
                .map_err(|_| EmbodimentError::InvalidInput("scene wardrobe order"))?;
            layers.push(SceneLayer::from_asset(
                LayerRole::Outfit,
                asset,
                LayerRole::Outfit.default_z().saturating_add(offset),
            ));
        }

        let expression = match expression {
            Some(name) => Some(resolve_motion(
                appearance,
                assets,
                name,
                &appearance.expressions,
                AssetKind::Expression,
            )?),
            None => None,
        };
        let gesture = match gesture {
            Some(name) => Some(resolve_motion(
                appearance,
                assets,
                name,
                &appearance.gestures,
                AssetKind::Gesture,
            )?),
            None => None,
        };

        Self::new(
            &appearance.core_uuid,
            appearance.theme.clone(),
            layers,
            expression,
            gesture,
        )
    }

    pub fn validate(&self) -> Result<(), EmbodimentError> {
        if self.schema != SCENE_SCHEMA {
            return Err(EmbodimentError::CorruptData("scene schema"));
        }
        if self.core_uuid.trim().is_empty() {
            return Err(EmbodimentError::InvalidInput("scene core uuid"));
        }
        validate_theme(&self.theme)?;
        if self.layers.len() > MAX_SCENE_LAYERS {
            return Err(EmbodimentError::InvalidInput("scene layer count"));
        }

        let mut base = 0usize;
        let mut hair = 0usize;
        let mut eyes = 0usize;
        let mut overlay = 0usize;
        for layer in &self.layers {
            layer.validate()?;
            match layer.role {
                LayerRole::Base => base += 1,
                LayerRole::Hair => hair += 1,
                LayerRole::Eyes => eyes += 1,
                LayerRole::Overlay => overlay += 1,
                LayerRole::Outfit => {}
            }
        }
        if base > 1 || hair > 1 || eyes > 1 || overlay > 1 {
            return Err(EmbodimentError::InvalidInput("scene duplicate layer role"));
        }

        if let Some(expression) = &self.expression {
            expression.validate()?;
            if expression.name.trim().is_empty() {
                return Err(EmbodimentError::InvalidInput("scene expression"));
            }
        }
        if let Some(gesture) = &self.gesture {
            gesture.validate()?;
            if gesture.name.trim().is_empty() {
                return Err(EmbodimentError::InvalidInput("scene gesture"));
            }
        }
        Ok(())
    }
}

fn resolve_motion(
    appearance: &Appearance,
    assets: &[Asset],
    name: &str,
    bindings: &std::collections::BTreeMap<String, String>,
    expected: AssetKind,
) -> Result<ActiveMotion, EmbodimentError> {
    validate_animation_name(name)?;
    let trimmed = name.trim();
    let asset_id = bindings.get(trimmed).ok_or(EmbodimentError::NotFound)?;
    require_asset(assets, &appearance.core_uuid, asset_id, expected)?;
    ActiveMotion::new(trimmed, asset_id)
}

fn require_asset<'a>(
    assets: &'a [Asset],
    core_uuid: &str,
    asset_id: &str,
    expected: AssetKind,
) -> Result<&'a Asset, EmbodimentError> {
    validate_reference(asset_id)?;
    let asset = assets
        .iter()
        .find(|asset| asset.asset_id == asset_id)
        .ok_or(EmbodimentError::NotFound)?;
    if asset.core_uuid != core_uuid {
        return Err(EmbodimentError::OwnershipMismatch);
    }
    if asset.deleted {
        return Err(EmbodimentError::Deleted);
    }
    if asset.kind != expected {
        return Err(EmbodimentError::WrongAssetKind(asset_id.to_owned()));
    }
    Ok(asset)
}

fn validate_reference(value: &str) -> Result<(), EmbodimentError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_SCENE_ASSET_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(EmbodimentError::InvalidInput("scene asset reference"));
    }
    Ok(())
}
