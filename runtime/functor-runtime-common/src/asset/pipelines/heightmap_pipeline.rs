use crate::asset::{AssetCache, AssetPipeline, AssetPipelineContext};
use serde::{Deserialize, Deserializer, Serialize};

/// CPU-resident 16-bit elevation samples. Keeping the source precision here
/// lets both the GPU terrain renderer and Rapier heightfield adapter consume
/// the exact same data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeightmapData {
    pub(crate) samples: Vec<u16>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Deterministic content fingerprint for fast renderer/cache invalidation.
    /// Semantic equality still includes the samples; `Arc<HeightmapData>`
    /// takes a pointer fast path for unchanged per-frame declarations.
    pub(crate) revision: u64,
}

impl<'de> Deserialize<'de> for HeightmapData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // `revision` is an optimization, never trusted wire input. Ignoring a
        // serialized revision and recomputing it keeps equality/content
        // identity sound across snapshots and external protocol values.
        #[derive(Deserialize)]
        struct Wire {
            samples: Vec<u16>,
            width: u32,
            height: u32,
        }

        let Wire {
            samples,
            width,
            height,
        } = Wire::deserialize(deserializer)?;
        Ok(Self {
            revision: revision(width, height, &samples),
            samples,
            width,
            height,
        })
    }
}

impl HeightmapData {
    pub fn flat() -> Self {
        let samples = vec![0; 4];
        Self {
            revision: revision(2, 2, &samples),
            samples,
            width: 2,
            height: 2,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.width >= 2
            && self.height >= 2
            && self.samples.len() == (self.width as usize * self.height as usize)
    }
}

/// Decode a grayscale heightmap without losing 16-bit PNG precision.
///
/// Color and 8-bit images are accepted and converted to 16-bit luminance, but
/// authored terrain should use 16-bit grayscale PNG. Decode failures become a
/// flat sentinel instead of panicking the render thread.
pub struct HeightmapPipeline;

impl AssetPipeline<HeightmapData> for HeightmapPipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        _context: AssetPipelineContext,
    ) -> HeightmapData {
        match image::load_from_memory(&bytes) {
            Ok(image) => {
                // Consume the decoded image. `to_luma16` clones even an
                // already-authored 16-bit grayscale source, which would add
                // another 32 MiB allocation for a 4096² heightmap.
                let image = image.into_luma16();
                let width = image.width();
                let height = image.height();
                if width < 2 || height < 2 {
                    eprintln!(
                        "[terrain] heightmap must be at least 2x2 pixels, got {width}x{height}"
                    );
                    return HeightmapData::flat();
                }
                let samples = image.into_raw();
                HeightmapData {
                    revision: revision(width, height, &samples),
                    width,
                    height,
                    samples,
                }
            }
            Err(error) => {
                eprintln!("[terrain] cannot decode heightmap: {error}");
                HeightmapData::flat()
            }
        }
    }

    fn unloaded_asset(&self, _context: AssetPipelineContext) -> HeightmapData {
        HeightmapData::flat()
    }
}

fn revision(width: u32, height: u32, samples: &[u16]) -> u64 {
    // FNV-1a: tiny, deterministic on every target, and paid once at decode.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in width
        .to_le_bytes()
        .into_iter()
        .chain(height.to_le_bytes())
        .chain(samples.iter().flat_map(|sample| sample.to_le_bytes()))
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, GrayImage, ImageBuffer, ImageFormat, Luma};

    use super::*;

    fn encode(image: DynamicImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn process(bytes: Vec<u8>) -> HeightmapData {
        HeightmapPipeline.process(bytes, &AssetCache::new(), AssetPipelineContext {})
    }

    #[test]
    fn preserves_sixteen_bit_height_samples_exactly() {
        let image: ImageBuffer<Luma<u16>, Vec<u16>> =
            ImageBuffer::from_raw(2, 2, vec![0, 1, 32768, 65535]).unwrap();
        let decoded = process(encode(DynamicImage::ImageLuma16(image)));
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(decoded.samples, [0, 1, 32768, 65535]);
    }

    #[test]
    fn expands_eight_bit_luminance_to_the_full_sixteen_bit_range() {
        let image = GrayImage::from_raw(2, 2, vec![0, 1, 128, 255]).unwrap();
        let decoded = process(encode(DynamicImage::ImageLuma8(image)));
        assert_eq!(decoded.samples, [0, 257, 32896, 65535]);
    }

    #[test]
    fn corrupt_image_becomes_a_flat_valid_fallback() {
        let decoded = process(vec![1, 2, 3, 4]);
        assert!(decoded.is_valid());
        assert!(decoded.samples.iter().all(|sample| *sample == 0));
    }

    #[test]
    fn undersized_images_become_a_flat_valid_fallback() {
        for (width, height) in [(1, 1), (1, 2), (2, 1)] {
            let image = GrayImage::from_pixel(width, height, Luma([255]));
            let decoded = process(encode(DynamicImage::ImageLuma8(image)));
            assert!(
                decoded.is_valid(),
                "{width}x{height} should use the valid fallback"
            );
            assert_eq!((decoded.width, decoded.height), (2, 2));
            assert!(decoded.samples.iter().all(|sample| *sample == 0));
        }
    }

    #[test]
    fn deserialization_recomputes_untrusted_content_revision() {
        let data = HeightmapData::flat();
        let forged = format!(
            r#"{{"samples":[0,0,0,0],"width":2,"height":2,"revision":{}}}"#,
            data.revision.wrapping_add(1)
        );
        let decoded: HeightmapData = serde_json::from_str(&forged).unwrap();
        assert_eq!(decoded, data);
        assert_eq!(decoded.revision, data.revision);
    }
}
