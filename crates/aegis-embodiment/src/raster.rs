//! Dependency-free software rasterizer that emits real 24-bit BMP pixels.
//!
//! This module is deliberately small and honest about what it does. It does
//! **not** decode image assets (PNG/JPEG), run a GPU, open a native window, or
//! perform lip-sync. It composites the *layers* of an [`AvatarScene`] as solid,
//! role-colored rectangles derived from the Core theme, applies the optional
//! animation transform, and writes a valid uncompressed 24-bit BMP file.
//!
//! Colors and geometry are deterministic functions of the scene and canvas
//! size, so identical input produces byte-for-byte identical output. This is
//! simple rectangle/layer compositing, not full artistic rendering.

use std::path::Path;

use crate::animation::{AnimationState, Transform};
use crate::error::EmbodimentError;
use crate::scene::{AvatarScene, LayerRole};

/// BMP file signature (`BM`).
pub const BMP_MAGIC: [u8; 2] = [b'B', b'M'];
/// Smallest raster dimension accepted by [`render_bmp`].
pub const MIN_BMP_DIMENSION: u32 = 16;
/// Largest raster dimension accepted by [`render_bmp`].
pub const MAX_BMP_DIMENSION: u32 = 2048;
/// Size of the file header (14 bytes) plus the DIB header (40 bytes).
pub const BMP_HEADER_BYTES: usize = 54;
/// Largest BMP accepted for writing (64 MiB).
pub const MAX_BMP_BYTES: usize = 64 * 1024 * 1024;

const BMP_DIB_HEADER_BYTES: u32 = 40;
const BMP_BITS_PER_PIXEL: u16 = 24;
const BMP_PIXELS_PER_METER: i32 = 2835;

/// An axis-aligned rectangle in top-down pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Render `scene` to a 24-bit BMP with no animation transform applied.
///
/// Equivalent to [`render_bmp_at`] with `animation = None`.
pub fn render_bmp(
    scene: &AvatarScene,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, EmbodimentError> {
    render_bmp_at(scene, width, height, None)
}

/// Render `scene` to a 24-bit BMP, applying an optional animation transform.
///
/// The background is derived from the Core theme `primary` color. Every visible
/// layer is composited as one or more role-colored rectangles. Active
/// expression and gesture references add visible accent/tertiary overlays. All
/// rectangles are clipped to the canvas.
pub fn render_bmp_at(
    scene: &AvatarScene,
    width: u32,
    height: u32,
    animation: Option<&AnimationState>,
) -> Result<Vec<u8>, EmbodimentError> {
    scene.validate()?;
    validate_dimensions(width, height)?;
    if scene.layers.is_empty() {
        return Err(EmbodimentError::InvalidInput("scene layers"));
    }

    let primary = parse_hex_color(&scene.theme.primary)?;
    let accent = parse_hex_color(&scene.theme.accent)?;
    let tertiary = parse_hex_color(&scene.theme.tertiary)?;
    let background = shade(primary, 40);

    let mut pixels = vec![0u8; raster_len(width, height)?];
    fill_rect(
        &mut pixels,
        width,
        height,
        Rect::new(0, 0, width as i32, height as i32),
        background,
    )?;

    let transform = animation.map_or_else(Transform::identity, |state| state.transform);
    let shift = shift_from(transform, width, height);

    let mut layers = scene.layers.clone();
    layers.sort_by(|left, right| {
        left.z_index
            .cmp(&right.z_index)
            .then(left.role.cmp(&right.role))
            .then(left.asset_id.cmp(&right.asset_id))
    });

    for layer in &layers {
        if !layer.visible {
            continue;
        }
        let color = role_color(layer.role, primary, accent, tertiary);
        let color = mix(color, background, opacity_weight(transform.opacity));
        for rect in layer_rects(layer.role, width, height) {
            let rect = apply_transform(rect, &shift, width, height);
            fill_rect(&mut pixels, width, height, rect, color)?;
        }
    }

    if scene.expression.is_some() {
        let rect = apply_transform(
            fraction_rect(0.42, 0.68, 0.16, 0.05, width, height),
            &shift,
            width,
            height,
        );
        let color = mix(accent, background, opacity_weight(transform.opacity));
        fill_rect(&mut pixels, width, height, rect, color)?;
    }
    if scene.gesture.is_some() {
        let rect = apply_transform(
            fraction_rect(0.80, 0.30, 0.14, 0.40, width, height),
            &shift,
            width,
            height,
        );
        let color = mix(tertiary, background, opacity_weight(transform.opacity));
        fill_rect(&mut pixels, width, height, rect, color)?;
    }

    encode_bmp(width, height, &pixels)
}

/// Write an already-encoded BMP to `path`, validating magic and file size.
pub fn write_bmp(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), EmbodimentError> {
    if bytes.len() > MAX_BMP_BYTES {
        return Err(EmbodimentError::InvalidInput("bmp size"));
    }
    if bytes.len() < BMP_HEADER_BYTES || bytes[0..2] != BMP_MAGIC {
        return Err(EmbodimentError::InvalidInput("bmp header"));
    }
    let declared = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
    if declared != bytes.len() {
        return Err(EmbodimentError::CorruptData("bmp declared size"));
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Fill `rect` with a solid RGB color, clipping to the `width` x `height` canvas.
///
/// `pixels` is a top-down, packed RGB8 buffer of exactly `width * height * 3`
/// bytes. Out-of-bounds and empty rectangles are ignored.
pub fn fill_rect(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    rect: Rect,
    color: [u8; 3],
) -> Result<(), EmbodimentError> {
    if width == 0 || height == 0 {
        return Err(EmbodimentError::InvalidInput("raster dimensions"));
    }
    if pixels.len() != raster_len(width, height)? {
        return Err(EmbodimentError::InvalidInput("raster buffer"));
    }
    if rect.width <= 0 || rect.height <= 0 {
        return Ok(());
    }

    let canvas_width = i64::from(width);
    let canvas_height = i64::from(height);
    let x0 = i64::from(rect.x).max(0);
    let y0 = i64::from(rect.y).max(0);
    let x1 = (i64::from(rect.x) + i64::from(rect.width)).min(canvas_width);
    let y1 = (i64::from(rect.y) + i64::from(rect.height)).min(canvas_height);
    if x1 <= x0 || y1 <= y0 {
        return Ok(());
    }

    for y in y0..y1 {
        let row = y as usize * width as usize * 3;
        for x in x0..x1 {
            let offset = row + x as usize * 3;
            pixels[offset..offset + 3].copy_from_slice(&color);
        }
    }
    Ok(())
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), EmbodimentError> {
    if !(MIN_BMP_DIMENSION..=MAX_BMP_DIMENSION).contains(&width)
        || !(MIN_BMP_DIMENSION..=MAX_BMP_DIMENSION).contains(&height)
    {
        return Err(EmbodimentError::InvalidInput("bmp dimensions"));
    }
    Ok(())
}

fn raster_len(width: u32, height: u32) -> Result<usize, EmbodimentError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(3))
        .ok_or(EmbodimentError::InvalidInput("raster buffer"))
}

fn encode_bmp(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, EmbodimentError> {
    let row_bytes = (width as usize)
        .checked_mul(3)
        .ok_or(EmbodimentError::InvalidInput("bmp size"))?;
    let stride = row_bytes.div_ceil(4) * 4;
    let pixel_bytes = stride
        .checked_mul(height as usize)
        .ok_or(EmbodimentError::InvalidInput("bmp size"))?;
    let file_size = BMP_HEADER_BYTES
        .checked_add(pixel_bytes)
        .ok_or(EmbodimentError::InvalidInput("bmp size"))?;
    if file_size > MAX_BMP_BYTES {
        return Err(EmbodimentError::InvalidInput("bmp size"));
    }

    let mut output = Vec::with_capacity(file_size);
    output.extend_from_slice(&BMP_MAGIC);
    output.extend_from_slice(&(file_size as u32).to_le_bytes());
    output.extend_from_slice(&[0, 0, 0, 0]);
    output.extend_from_slice(&(BMP_HEADER_BYTES as u32).to_le_bytes());

    output.extend_from_slice(&BMP_DIB_HEADER_BYTES.to_le_bytes());
    output.extend_from_slice(&(width as i32).to_le_bytes());
    output.extend_from_slice(&(height as i32).to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&BMP_BITS_PER_PIXEL.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    output.extend_from_slice(&BMP_PIXELS_PER_METER.to_le_bytes());
    output.extend_from_slice(&BMP_PIXELS_PER_METER.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());
    output.extend_from_slice(&0u32.to_le_bytes());

    let padding = stride - row_bytes;
    for y in (0..height as usize).rev() {
        let row_start = y * row_bytes;
        for x in 0..width as usize {
            let offset = row_start + x * 3;
            output.push(pixels[offset + 2]);
            output.push(pixels[offset + 1]);
            output.push(pixels[offset]);
        }
        if padding > 0 {
            output.resize(output.len() + padding, 0);
        }
    }

    Ok(output)
}

fn parse_hex_color(value: &str) -> Result<[u8; 3], EmbodimentError> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return Err(EmbodimentError::InvalidInput("theme color"));
    }
    let mut color = [0u8; 3];
    for (index, chunk) in bytes[1..].chunks_exact(2).enumerate() {
        let high = hex_nibble(chunk[0]).ok_or(EmbodimentError::InvalidInput("theme color"))?;
        let low = hex_nibble(chunk[1]).ok_or(EmbodimentError::InvalidInput("theme color"))?;
        color[index] = (high << 4) | low;
    }
    Ok(color)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn shade(color: [u8; 3], factor: u8) -> [u8; 3] {
    let scale = u32::from(factor);
    [
        ((u32::from(color[0]) * scale) / 255) as u8,
        ((u32::from(color[1]) * scale) / 255) as u8,
        ((u32::from(color[2]) * scale) / 255) as u8,
    ]
}

fn mix(front: [u8; 3], back: [u8; 3], weight: u16) -> [u8; 3] {
    let front_weight = u32::from(weight.min(255));
    let back_weight = 255 - front_weight;
    [
        ((u32::from(front[0]) * front_weight + u32::from(back[0]) * back_weight) / 255) as u8,
        ((u32::from(front[1]) * front_weight + u32::from(back[1]) * back_weight) / 255) as u8,
        ((u32::from(front[2]) * front_weight + u32::from(back[2]) * back_weight) / 255) as u8,
    ]
}

fn opacity_weight(opacity: f32) -> u16 {
    let clamped = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    };
    (clamped * 255.0).round() as u16
}

fn role_color(role: LayerRole, primary: [u8; 3], accent: [u8; 3], tertiary: [u8; 3]) -> [u8; 3] {
    match role {
        LayerRole::Base => primary,
        LayerRole::Hair => shade(primary, 170),
        LayerRole::Eyes => accent,
        LayerRole::Outfit => tertiary,
        LayerRole::Overlay => mix(accent, primary, 180),
    }
}

fn layer_rects(role: LayerRole, width: u32, height: u32) -> Vec<Rect> {
    match role {
        LayerRole::Base => vec![fraction_rect(0.20, 0.18, 0.60, 0.82, width, height)],
        LayerRole::Outfit => vec![fraction_rect(0.25, 0.45, 0.50, 0.45, width, height)],
        LayerRole::Hair => vec![fraction_rect(0.25, 0.08, 0.50, 0.25, width, height)],
        LayerRole::Eyes => vec![
            fraction_rect(0.40, 0.30, 0.08, 0.06, width, height),
            fraction_rect(0.52, 0.30, 0.08, 0.06, width, height),
        ],
        LayerRole::Overlay => vec![fraction_rect(0.02, 0.02, 0.96, 0.05, width, height)],
    }
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

#[derive(Debug, Clone, Copy)]
struct Shift {
    x: i32,
    y: i32,
    scale: f32,
}

fn shift_from(transform: Transform, width: u32, height: u32) -> Shift {
    let translate_x = finite_or(transform.translate_x, 0.0).clamp(-1.0, 1.0);
    let translate_y = finite_or(transform.translate_y, 0.0).clamp(-1.0, 1.0);
    let scale = finite_or(transform.scale, 1.0).clamp(0.1, 4.0);
    Shift {
        x: (translate_x * width as f32).round() as i32,
        y: (translate_y * height as f32).round() as i32,
        scale,
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn apply_transform(rect: Rect, shift: &Shift, width: u32, height: u32) -> Rect {
    if (shift.scale - 1.0).abs() < f32::EPSILON && shift.x == 0 && shift.y == 0 {
        return rect;
    }
    let canvas_x = width as f32 / 2.0;
    let canvas_y = height as f32 / 2.0;
    let width_f = rect.width as f32 * shift.scale;
    let height_f = rect.height as f32 * shift.scale;
    let center_x = rect.x as f32 + rect.width as f32 / 2.0;
    let center_y = rect.y as f32 + rect.height as f32 / 2.0;
    let scaled_center_x = canvas_x + (center_x - canvas_x) * shift.scale;
    let scaled_center_y = canvas_y + (center_y - canvas_y) * shift.scale;
    Rect {
        x: (scaled_center_x - width_f / 2.0).round() as i32 + shift.x,
        y: (scaled_center_y - height_f / 2.0).round() as i32 + shift.y,
        width: width_f.round() as i32,
        height: height_f.round() as i32,
    }
}
