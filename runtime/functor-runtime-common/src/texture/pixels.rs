//! Textures built from decoded pixels (`Texture.fromPixels`): validated and
//! converted to RGBA8 once, then uploaded on first draw by the render side's
//! cache (keyed by [`PixelData::id`]).

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::{PixelFormat, TextureData};

/// The largest side accepted: what every GPU we ship on (desktop, browsers,
/// Quest) supports in practice, though GLES 3 / WebGL2 promise only 2048.
pub const MAX_TEXTURE_SIZE: u32 = 4096;

/// How the input bytes encode a pixel. Rows run top first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelEncoding {
    Rgba8,
    Rgb8,
    /// A little-endian u16: red in bits 0–4, green 5–9, blue 10–14; bit 15
    /// is ignored. Channels expand to the full 0–255 range.
    Rgb555le,
}

impl PixelEncoding {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelEncoding::Rgba8 => 4,
            PixelEncoding::Rgb8 => 3,
            PixelEncoding::Rgb555le => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            PixelEncoding::Rgba8 => "Rgba8",
            PixelEncoding::Rgb8 => "Rgb8",
            PixelEncoding::Rgb555le => "Rgb555le",
        }
    }
}

/// What the texels mean. `Srgb` is colour (decoded to linear when sampled,
/// mipmapped, repeat-wrapped); `Linear` is data such as lightmaps (sampled as
/// stored, no mipmaps, clamped to the edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSpace {
    Srgb,
    Linear,
}

/// Validated RGBA8 pixels, top row first. Immutable; equality is identity.
pub struct PixelData {
    id: u64,
    color_space: ColorSpace,
    data: TextureData,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl PixelData {
    /// Validate `pixels` against the size and encoding, then convert.
    pub fn new(
        width: u32,
        height: u32,
        encoding: PixelEncoding,
        color_space: ColorSpace,
        pixels: &[u8],
    ) -> Result<PixelData, String> {
        check_side("width", width as f64)?;
        check_side("height", height as f64)?;
        let need = width as usize * height as usize * encoding.bytes_per_pixel();
        if pixels.len() != need {
            return Err(format!(
                "pixels: {} bytes, but {width}×{height} {} needs {need} ({} per pixel)",
                pixels.len(),
                encoding.name(),
                encoding.bytes_per_pixel()
            ));
        }
        let rgba8 = match encoding {
            PixelEncoding::Rgba8 => pixels.to_vec(),
            PixelEncoding::Rgb8 => pixels
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            PixelEncoding::Rgb555le => pixels
                .chunks_exact(2)
                .flat_map(|p| {
                    let v = u16::from_le_bytes([p[0], p[1]]);
                    // 5 → 8 bits by replicating the high bits: 0 → 0, 31 → 255.
                    let expand = |c: u16| ((c << 3) | (c >> 2)) as u8;
                    [
                        expand(v & 31),
                        expand((v >> 5) & 31),
                        expand((v >> 10) & 31),
                        255,
                    ]
                })
                .collect(),
        };
        Ok(Self::from_rgba8(width, height, color_space, rgba8))
    }

    /// Wrap already-checked RGBA8 pixels with a fresh identity.
    pub(super) fn from_rgba8(width: u32, height: u32, color_space: ColorSpace, rgba8: Vec<u8>) -> PixelData {
        PixelData {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            color_space,
            data: TextureData {
                bytes: rgba8,
                width,
                height,
                format: PixelFormat::RGBA,
            },
        }
    }

    /// Unique per built texture, never reused: the render-side cache key.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.data.width
    }

    pub fn height(&self) -> u32 {
        self.data.height
    }

    pub fn color_space(&self) -> ColorSpace {
        self.color_space
    }

    /// Four bytes per pixel, top row first.
    pub fn rgba8(&self) -> &[u8] {
        &self.data.bytes
    }

    /// The pixels as upload input.
    pub fn texture_data(&self) -> &TextureData {
        &self.data
    }
}

/// A side length as a whole number of texels, `1..=MAX_TEXTURE_SIZE`.
pub fn check_side(name: &str, side: f64) -> Result<u32, String> {
    if side.fract() != 0.0 {
        return Err(format!("{name} must be a whole number, got {side}"));
    }
    if !(1.0..=MAX_TEXTURE_SIZE as f64).contains(&side) {
        return Err(format!(
            "{name} must be 1 to {MAX_TEXTURE_SIZE} (the largest texture every target \
supports), got {side}"
        ));
    }
    Ok(side as u32)
}

impl PartialEq for PixelData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Debug for PixelData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PixelData")
            .field("id", &self.id)
            .field("width", &self.data.width)
            .field("height", &self.data.height)
            .field("color_space", &self.color_space)
            .finish()
    }
}

/// The wire form: size, colour space and RGBA8 bytes inline.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PixelWire<B> {
    width: u32,
    height: u32,
    color_space: ColorSpace,
    rgba8: B,
}

impl Serialize for PixelData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PixelWire {
            width: self.width(),
            height: self.height(),
            color_space: self.color_space,
            rgba8: self.rgba8(),
        }
        .serialize(serializer)
    }
}

// Reading it back validates and gives a fresh identity.
impl<'de> Deserialize<'de> for PixelData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let w = PixelWire::<Vec<u8>>::deserialize(deserializer)?;
        let checked = check_side("width", w.width as f64)
            .and(check_side("height", w.height as f64))
            .and_then(|_| match w.width as usize * w.height as usize * 4 {
                need if need == w.rgba8.len() => Ok(()),
                need => Err(format!("rgba8: {} bytes, need {need}", w.rgba8.len())),
            });
        checked.map_err(serde::de::Error::custom)?;
        Ok(PixelData::from_rgba8(
            w.width,
            w.height,
            w.color_space,
            w.rgba8,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(encoding: PixelEncoding, width: u32, bytes: &[u8]) -> Vec<u8> {
        let height = (bytes.len() / encoding.bytes_per_pixel()) as u32 / width;
        PixelData::new(width, height, encoding, ColorSpace::Linear, bytes)
            .expect("valid")
            .rgba8()
            .to_vec()
    }

    #[test]
    fn each_encoding_decodes_to_rgba8() {
        assert_eq!(rgba(PixelEncoding::Rgba8, 1, &[1, 2, 3, 4]), [1, 2, 3, 4]);
        assert_eq!(
            rgba(PixelEncoding::Rgb8, 2, &[1, 2, 3, 4, 5, 6]),
            [1, 2, 3, 255, 4, 5, 6, 255]
        );
        // Red 31, green 0, blue 16 (bit 15 set, ignored); then green 1.
        let red = 31u16 | (16 << 10) | (1 << 15);
        let green = 1u16 << 5;
        let bytes: Vec<u8> = [red, green].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(
            rgba(PixelEncoding::Rgb555le, 2, &bytes),
            [255, 0, 132, 255, 0, 8, 0, 255]
        );
    }

    #[test]
    fn errors_name_the_problem() {
        let err = |w, h, enc, n| {
            PixelData::new(w, h, enc, ColorSpace::Srgb, &vec![0; n]).expect_err("invalid")
        };
        assert_eq!(
            err(2, 2, PixelEncoding::Rgb8, 10),
            "pixels: 10 bytes, but 2×2 Rgb8 needs 12 (3 per pixel)"
        );
        assert_eq!(
            err(0, 2, PixelEncoding::Rgba8, 0),
            "width must be 1 to 4096 (the largest texture every target supports), got 0"
        );
        assert_eq!(
            err(1, 4097, PixelEncoding::Rgb555le, 8194),
            "height must be 1 to 4096 (the largest texture every target supports), got 4097"
        );
    }

    #[test]
    fn wire_round_trip_keeps_pixels_and_gets_a_fresh_identity() {
        let a = PixelData::new(1, 1, PixelEncoding::Rgb8, ColorSpace::Linear, &[9, 8, 7]).unwrap();
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(
            json,
            r#"{"width":1,"height":1,"colorSpace":"Linear","rgba8":[9,8,7,255]}"#
        );
        let b: PixelData = serde_json::from_str(&json).unwrap();
        assert_eq!(
            (b.rgba8(), b.color_space()),
            (a.rgba8(), ColorSpace::Linear)
        );
        assert_ne!(a.id(), b.id());
        let bad = r#"{"width":2,"height":1,"colorSpace":"Srgb","rgba8":[1,2,3,4]}"#;
        assert!(serde_json::from_str::<PixelData>(bad).is_err());
    }
}
