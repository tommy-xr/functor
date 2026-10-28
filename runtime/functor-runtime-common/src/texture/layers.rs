//! Switchable layers on an atlas (`Texture.withLayers`): each packed image
//! may carry layers of the same size, mixed in as `base + Σ intensity ×
//! layer` (saturating). Composition runs on the CPU per image, so a change
//! re-uploads only the images whose intensities changed.

use serde::{Deserialize, Serialize};

use super::PixelData;

/// One switchable contribution: RGBA8, the image's size, top row first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: f64,
    pub rgba8: Vec<u8>,
}

/// An atlas image with layers: its first texel (`x`, `y`), size, the atlas
/// padding around it, and its layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayeredImage {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub padding: u32,
    pub layers: Vec<Layer>,
}

/// One region to upload: its corner in the atlas, size and RGBA8 texels.
pub struct Block {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// Each layer's intensity, flattened across `images` in order. `intensities`
/// is sorted by id; an id it lacks is 0.
pub fn layer_levels(images: &[LayeredImage], intensities: &[(f64, f32)]) -> Vec<f32> {
    images
        .iter()
        .flat_map(|image| &image.layers)
        .map(|layer| {
            intensities
                .binary_search_by(|(id, _)| id.total_cmp(&layer.id))
                .map_or(0.0, |i| intensities[i].1)
        })
        .collect()
}

/// `image`'s padded block of `atlas` at `levels` (one per layer): the base
/// plus each layer × level, rounded and clamped to 255 (alpha stays the
/// base's), with its padding regenerated from the composed edge.
pub fn compose_block(atlas: &PixelData, image: &LayeredImage, levels: &[f32]) -> Block {
    let (w, h) = (image.width as usize, image.height as usize);
    let side = atlas.width() as usize;
    let mut sum: Vec<f32> = (0..h)
        .flat_map(|row| {
            let start = ((image.y as usize + row) * side + image.x as usize) * 4;
            atlas.rgba8()[start..start + w * 4].iter().map(|&b| b as f32)
        })
        .collect();
    for (layer, &level) in image.layers.iter().zip(levels) {
        if level == 0.0 {
            continue;
        }
        for (i, (s, &l)) in sum.iter_mut().zip(&layer.rgba8).enumerate() {
            if i % 4 != 3 {
                *s += level * l as f32;
            }
        }
    }
    let texels: Vec<u8> = sum.iter().map(|s| s.round().min(255.0) as u8).collect();
    // The padded block: each texel repeats its nearest composed one.
    let pad = image.padding as i64;
    let (pw, ph) = (w + 2 * pad as usize, h + 2 * pad as usize);
    let mut rgba8 = Vec::with_capacity(pw * ph * 4);
    for row in -pad..h as i64 + pad {
        let sy = row.clamp(0, h as i64 - 1) as usize;
        for col in -pad..w as i64 + pad {
            let i = (sy * w + col.clamp(0, w as i64 - 1) as usize) * 4;
            rgba8.extend_from_slice(&texels[i..i + 4]);
        }
    }
    Block {
        x: image.x - image.padding,
        y: image.y - image.padding,
        width: pw as u32,
        height: ph as u32,
        rgba8,
    }
}

/// Check `images` against an atlas `side` texels square: padded rects in
/// bounds, layer ids finite and distinct per image, layer sizes matching.
pub fn check_layers(images: &[LayeredImage], side: u32) -> Result<(), String> {
    for (i, image) in images.iter().enumerate() {
        let fits = |at: u32, len: u32| {
            at >= image.padding && at as u64 + len as u64 + image.padding as u64 <= side as u64
        };
        if image.width == 0 || image.height == 0 || !fits(image.x, image.width) || !fits(image.y, image.height) {
            return Err(format!("layered image {i} is outside the {side}×{side} atlas"));
        }
        let need = image.width as usize * image.height as usize * 4;
        for (j, layer) in image.layers.iter().enumerate() {
            if !layer.id.is_finite() {
                return Err(format!("layers[{j}]: id must be finite, got {}", layer.id));
            }
            if let Some(k) = image.layers[..j].iter().position(|l| l.id == layer.id) {
                return Err(format!("layers[{j}]: id {} repeats layers[{k}]", layer.id));
            }
            if layer.rgba8.len() != need {
                return Err(format!("layers[{j}]: {} RGBA8 bytes, need {need}", layer.rgba8.len()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::{ColorSpace, PixelEncoding};

    /// A 4×4 atlas, red 10 everywhere, with a 2×1 image at (1, 1), padding 1.
    fn atlas() -> (PixelData, LayeredImage) {
        let base = PixelData::new(4, 4, PixelEncoding::Rgba8, ColorSpace::Linear, &[10; 64]).unwrap();
        let layer = |id: f64, v: u8| Layer { id, rgba8: vec![v, v, v, 200, v, 0, 0, 200] };
        let image = LayeredImage {
            x: 1,
            y: 1,
            width: 2,
            height: 1,
            padding: 1,
            layers: vec![layer(7.0, 100), layer(2.0, 200)],
        };
        (base, image)
    }

    #[test]
    fn levels_look_up_ids_and_default_to_zero() {
        let (_, image) = atlas();
        assert_eq!(layer_levels(&[image.clone()], &[(2.0, 0.25)]), [0.0, 0.25]);
        assert_eq!(layer_levels(&[image.clone(), image], &[(2.0, 1.0), (7.0, 0.5), (9.0, 1.0)]), [0.5, 1.0, 0.5, 1.0]);
    }

    /// 0, 0.5 and 1 give base, base + half and base + all; the sum saturates
    /// and alpha stays the base's.
    #[test]
    fn composes_base_plus_scaled_layers() {
        let (base, image) = atlas();
        let first = |levels: &[f32]| compose_block(&base, &image, levels).rgba8[4..12].to_vec();
        assert_eq!(first(&[0.0, 0.0]), [10, 10, 10, 10, 10, 10, 10, 10]);
        assert_eq!(first(&[0.5, 0.0]), [60, 60, 60, 10, 60, 10, 10, 10]);
        assert_eq!(first(&[1.0, 0.0]), [110, 110, 110, 10, 110, 10, 10, 10]);
        assert_eq!(first(&[1.0, 1.0]), [255, 255, 255, 10, 255, 10, 10, 10]);
    }

    /// The block covers the padded rect, its border copying the composed edge.
    #[test]
    fn padding_is_regenerated() {
        let (base, image) = atlas();
        let block = compose_block(&base, &image, &[1.0, 0.0]);
        assert_eq!((block.x, block.y, block.width, block.height), (0, 0, 4, 3));
        let left = [110, 110, 110, 10];
        let right = [110, 10, 10, 10];
        let row = [left, left, right, right].concat();
        assert_eq!(block.rgba8, [row.clone(), row.clone(), row].concat());
    }

    #[test]
    fn errors_name_the_problem() {
        let (_, image) = atlas();
        let check = |edit: fn(&mut LayeredImage), side| {
            let mut image = image.clone();
            edit(&mut image);
            check_layers(&[image], side)
        };
        assert_eq!(check(|_| {}, 4), Ok(()));
        assert_eq!(check(|_| {}, 3), Err("layered image 0 is outside the 3×3 atlas".into()));
        assert_eq!(check(|i| i.layers[1].id = 7.0, 4), Err("layers[1]: id 7 repeats layers[0]".into()));
        assert_eq!(check(|i| i.layers[0].id = f64::NAN, 4), Err("layers[0]: id must be finite, got NaN".into()));
        assert_eq!(check(|i| i.layers[0].rgba8.pop().map_or((), drop), 4), Err("layers[0]: 7 RGBA8 bytes, need 8".into()));
    }
}
