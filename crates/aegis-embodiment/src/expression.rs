//! Deterministic expression and gesture parameter mapping.
//!
//! [`expression_params`] and [`gesture_params`] are pure maps from a small enum
//! to synthetic overlay colors and numeric parameters. They do **not** encode any
//! Core identity, approved Core appearance, or real character art; the colors are
//! generic placeholders chosen so different states are visually distinguishable.
//!
//! [`render_expression_bmp`] renders a scene with the existing CPU BMP
//! rasterizer, then blends the mapped expression overlay into the mouth region.
//! It is still schematic rectangle compositing, not GPU rendering, decoded art,
//! rigging, or a windowed GUI.

use serde::{Deserialize, Serialize};

use crate::animation::AnimationState;
use crate::error::EmbodimentError;
use crate::lipsync::MouthShape;
use crate::raster::{render_bmp, render_bmp_at, Rect, BMP_HEADER_BYTES, BMP_MAGIC};
use crate::scene::AvatarScene;

/// Small synthetic expression set used for deterministic overlay mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expression {
    Neutral,
    Happy,
    Sad,
    Surprised,
    Angry,
    Thinking,
}

impl Expression {
    /// All expressions in stable order.
    pub const ALL: [Expression; 6] = [
        Expression::Neutral,
        Expression::Happy,
        Expression::Sad,
        Expression::Surprised,
        Expression::Angry,
        Expression::Thinking,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Happy => "happy",
            Self::Sad => "sad",
            Self::Surprised => "surprised",
            Self::Angry => "angry",
            Self::Thinking => "thinking",
        }
    }
}

/// Synthetic numeric parameters derived from an [`Expression`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpressionParams {
    pub expression: Expression,
    /// Overlay RGB color blended into the mouth region.
    pub overlay: [u8; 3],
    /// Blend weight of the overlay in `0..=255` (0 keeps the underlying pixel).
    pub overlay_weight: u8,
    /// Schematic mouth openness in `0..=100`.
    pub mouth_open: u8,
    /// Schematic eye openness in `0..=100`.
    pub eye_open: u8,
    /// Schematic brow tilt in `-100..=100`.
    pub brow_tilt: i8,
}

/// Pure map from an expression to its synthetic overlay parameters.
pub const fn expression_params(expression: Expression) -> ExpressionParams {
    let (overlay, overlay_weight, mouth_open, eye_open, brow_tilt) = match expression {
        Expression::Neutral => ([192, 192, 192], 120, 10, 80, 0),
        Expression::Happy => ([255, 215, 0], 200, 70, 65, 10),
        Expression::Sad => ([70, 90, 160], 180, 15, 45, -15),
        Expression::Surprised => ([255, 140, 0], 210, 90, 100, 20),
        Expression::Angry => ([210, 40, 40], 210, 50, 55, -20),
        Expression::Thinking => ([120, 200, 255], 170, 25, 70, 8),
    };
    ExpressionParams {
        expression,
        overlay,
        overlay_weight,
        mouth_open,
        eye_open,
        brow_tilt,
    }
}

/// Small synthetic gesture set used for deterministic parameter mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    None,
    Wave,
    Nod,
    Point,
    Shrug,
    ThumbsUp,
}

impl Gesture {
    /// All gestures in stable order.
    pub const ALL: [Gesture; 6] = [
        Gesture::None,
        Gesture::Wave,
        Gesture::Nod,
        Gesture::Point,
        Gesture::Shrug,
        Gesture::ThumbsUp,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Wave => "wave",
            Self::Nod => "nod",
            Self::Point => "point",
            Self::Shrug => "shrug",
            Self::ThumbsUp => "thumbs_up",
        }
    }
}

/// Synthetic numeric parameters derived from a [`Gesture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GestureParams {
    pub gesture: Gesture,
    pub overlay: [u8; 3],
    pub overlay_weight: u8,
    /// Schematic horizontal offset in pixels at a 128-pixel reference width.
    pub offset_x: i16,
    /// Schematic vertical offset in pixels at a 128-pixel reference height.
    pub offset_y: i16,
    /// Schematic scale in percent.
    pub scale_percent: u16,
}

/// Pure map from a gesture to its synthetic parameters.
pub const fn gesture_params(gesture: Gesture) -> GestureParams {
    let (overlay, overlay_weight, offset_x, offset_y, scale_percent) = match gesture {
        Gesture::None => ([0, 0, 0], 0, 0, 0, 100),
        Gesture::Wave => ([0, 194, 255], 180, 20, -10, 105),
        Gesture::Nod => ([57, 255, 20], 160, 0, 12, 100),
        Gesture::Point => ([255, 215, 0], 180, 30, 0, 100),
        Gesture::Shrug => ([200, 200, 200], 150, 0, 6, 100),
        Gesture::ThumbsUp => ([57, 255, 20], 190, 18, -14, 108),
    };
    GestureParams {
        gesture,
        overlay,
        overlay_weight,
        offset_x,
        offset_y,
        scale_percent,
    }
}

/// Backing overrides painted on top of a rasterized avatar scene.
///
/// The expression and gesture select synthetic overlay colors; `mouth` is an
/// optional lip-sync shape that paints one extra schematic mouth rectangle.
/// None of these values are decoded art or rig data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateOverlay {
    pub expression: Expression,
    pub gesture: Gesture,
    pub mouth: Option<MouthShape>,
}

impl StateOverlay {
    pub const fn new(expression: Expression, gesture: Gesture, mouth: Option<MouthShape>) -> Self {
        Self {
            expression,
            gesture,
            mouth,
        }
    }
}

/// Render `scene` to a BMP and blend the mapped expression overlay over the
/// schematic mouth region.
pub fn render_expression_bmp(
    scene: &AvatarScene,
    expression: Expression,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, EmbodimentError> {
    let mut bytes = render_bmp(scene, width, height)?;
    paint_expression(&mut bytes, expression, width, height)?;
    Ok(bytes)
}

/// Render a scene with the optional animation transform, then paint the
/// expression, gesture, and optional lip-sync mouth overlays.
///
/// This is the state-aware frame renderer used by [`crate::sequence`]. It is
/// still schematic CPU rectangle compositing: no art decoding, GPU, native
/// window, or audio is involved.
pub fn render_state_bmp(
    scene: &AvatarScene,
    overlay: &StateOverlay,
    width: u32,
    height: u32,
    animation: Option<&AnimationState>,
) -> Result<Vec<u8>, EmbodimentError> {
    let mut bytes = render_bmp_at(scene, width, height, animation)?;
    paint_expression(&mut bytes, overlay.expression, width, height)?;
    paint_gesture(&mut bytes, overlay.gesture, width, height)?;
    if let Some(shape) = overlay.mouth {
        paint_mouth(&mut bytes, shape, width, height)?;
    }
    Ok(bytes)
}

fn paint_expression(
    bytes: &mut [u8],
    expression: Expression,
    width: u32,
    height: u32,
) -> Result<(), EmbodimentError> {
    let params = expression_params(expression);
    let rect = expression_overlay_rect(width, height);
    paint_bmp_rect(bytes, rect, params.overlay, params.overlay_weight)
}

fn paint_gesture(
    bytes: &mut [u8],
    gesture: Gesture,
    width: u32,
    height: u32,
) -> Result<(), EmbodimentError> {
    let params = gesture_params(gesture);
    if params.overlay_weight == 0 {
        return Ok(());
    }
    let rect = gesture_overlay_rect(width, height);
    paint_bmp_rect(bytes, rect, params.overlay, params.overlay_weight)
}

fn paint_mouth(
    bytes: &mut [u8],
    shape: MouthShape,
    width: u32,
    height: u32,
) -> Result<(), EmbodimentError> {
    let (rect, color, weight) = match shape {
        MouthShape::Closed => return Ok(()),
        MouthShape::Slight => (
            fraction_rect(0.44, 0.69, 0.12, 0.035, width, height),
            [180, 120, 120],
            180,
        ),
        MouthShape::Open => (
            fraction_rect(0.42, 0.68, 0.16, 0.060, width, height),
            [110, 40, 40],
            210,
        ),
        MouthShape::Wide => (
            fraction_rect(0.40, 0.66, 0.20, 0.070, width, height),
            [150, 60, 60],
            200,
        ),
    };
    paint_bmp_rect(bytes, rect, color, weight)
}

fn expression_overlay_rect(width: u32, height: u32) -> Rect {
    fraction_rect(0.38, 0.62, 0.24, 0.10, width, height)
}

fn gesture_overlay_rect(width: u32, height: u32) -> Rect {
    fraction_rect(0.78, 0.30, 0.16, 0.36, width, height)
}

fn fraction_rect(
    x: f32,
    y: f32,
    width_fraction: f32,
    height_fraction: f32,
    width: u32,
    height: u32,
) -> Rect {
    Rect {
        x: (x * width as f32).round() as i32,
        y: (y * height as f32).round() as i32,
        width: (width_fraction * width as f32).round() as i32,
        height: (height_fraction * height as f32).round() as i32,
    }
}

fn paint_bmp_rect(
    bytes: &mut [u8],
    rect: Rect,
    overlay: [u8; 3],
    weight: u8,
) -> Result<(), EmbodimentError> {
    if bytes.len() < BMP_HEADER_BYTES || bytes[0..2] != BMP_MAGIC {
        return Err(EmbodimentError::CorruptData("expression bmp header"));
    }
    let declared = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
    if declared != bytes.len() {
        return Err(EmbodimentError::CorruptData("expression bmp size"));
    }
    let data_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    let height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
    let bits = u16::from_le_bytes([bytes[28], bytes[29]]);
    if width <= 0 || height <= 0 || bits != 24 {
        return Err(EmbodimentError::CorruptData("expression bmp layout"));
    }
    let width = width as usize;
    let height = height as usize;
    let stride = (width * 3).div_ceil(4) * 4;
    let expected = data_offset
        .checked_add(
            stride
                .checked_mul(height)
                .ok_or(EmbodimentError::InvalidInput("expression bmp size"))?,
        )
        .ok_or(EmbodimentError::InvalidInput("expression bmp size"))?;
    if expected > bytes.len() {
        return Err(EmbodimentError::CorruptData("expression bmp pixels"));
    }

    if rect.width <= 0 || rect.height <= 0 {
        return Ok(());
    }
    let x0 = i64::from(rect.x).max(0);
    let y0 = i64::from(rect.y).max(0);
    let x1 = (i64::from(rect.x) + i64::from(rect.width)).min(width as i64);
    let y1 = (i64::from(rect.y) + i64::from(rect.height)).min(height as i64);
    if x1 <= x0 || y1 <= y0 {
        return Ok(());
    }

    let weight = u32::from(weight);
    let inverse = 255 - weight;
    for y in y0..y1 {
        let file_row = height - 1 - y as usize;
        let row_start = data_offset + file_row * stride;
        for x in x0..x1 {
            let offset = row_start + x as usize * 3;
            for channel in 0..3 {
                let source = u32::from(overlay[2 - channel]);
                let base = u32::from(bytes[offset + channel]);
                bytes[offset + channel] = ((source * weight + base * inverse) / 255) as u8;
            }
        }
    }
    Ok(())
}
