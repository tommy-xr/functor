use crate::{
    asset::{AssetCache, AssetPipeline},
    texture::{
        apply_color_key, decode_image, Texture2D, TextureData, TextureOptions, TERRAIN_DETAIL_ANISOTROPY,
    },
};

/// Decode an image, or `None` if the bytes are not one we handle.
///
/// Unhandled formats and corrupt bytes both fall back to the checkerboard,
/// never a panic: asset hot-reload can catch a file mid-write, and the render
/// thread polls asset futures (a panic aborts the runtime). The next save
/// reloads it again.
fn decode(bytes: &Vec<u8>) -> Option<TextureData> {
    match decode_image(bytes) {
        Ok(mut data) => {
            apply_color_key(&mut data.bytes, data.width, data.height);
            Some(data)
        }
        Err(e) => {
            eprintln!("[texture] cannot decode image: {e}");
            None
        }
    }
}

/// Image files decoded for sampling. `srgb` picks the upload space: COLOR
/// images (albedo) store sRGB and decode to linear in hardware; DATA images
/// (normal maps) upload LINEAR — their texels are vectors, and an sRGB decode
/// would bend every normal (see `crate::color_space`).
pub struct TexturePipeline {
    pub srgb: bool,
}

impl AssetPipeline<Texture2D> for TexturePipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        context: crate::asset::AssetPipelineContext,
    ) -> Texture2D {
        let Some(texture_data) = decode(&bytes) else {
            return self.unloaded_asset(context);
        };
        Texture2D::init_from_data(
            texture_data,
            // Default to `wrap: true` so that model textures load correctly
            // TODO: Consider how to pass options to pipeline
            TextureOptions {
                wrap: true,
                linear: true,
                // Loaded textures are sampled on world geometry, which is
                // usually minified: without a mip chain a distant surface
                // samples one texel per fragment and crawls as the camera
                // moves. The fallback below deliberately keeps level 0 so a
                // missing asset still reads as a flat marker.
                mipmap: true,
                srgb: self.srgb,
                ..TextureOptions::default()
            },
        )
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        if self.srgb {
            let texture_data = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
            Texture2D::init_from_data(
                texture_data,
                TextureOptions {
                    srgb: true,
                    ..TextureOptions::default()
                },
            )
        } else {
            // A flat "straight up" tangent-space normal: the identity stand-in
            // for a missing/undecodable normal map (a color checkerboard would
            // dent the surface it stands in for).
            let texture_data = TextureData::solid_color([128, 128, 255, 255]);
            Texture2D::init_from_data(texture_data, TextureOptions::default())
        }
    }
}

/// Terrain detail maps: tiled, sampled almost entirely at grazing angles, and
/// normalized in the shader against their own mean.
///
/// A separate pipeline rather than a flag on the shared one, so the anisotropy
/// reduction and the mean scan apply to exactly the four maps that need them
/// and to nothing else in the engine. The asset cache keys decoded assets per
/// pipeline (bytes are still shared), so this costs no extra download.
pub struct TerrainDetailPipeline;

impl AssetPipeline<Texture2D> for TerrainDetailPipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        context: crate::asset::AssetPipelineContext,
    ) -> Texture2D {
        let Some(texture_data) = decode(&bytes) else {
            return self.unloaded_asset(context);
        };
        Texture2D::init_from_data(
            texture_data,
            TextureOptions {
                wrap: true,
                linear: true,
                mipmap: true,
                anisotropy: TERRAIN_DETAIL_ANISOTROPY,
                compute_average: true,
                // Detail maps are color; `compute_average` follows the space
                // `texture()` returns, so the divisor stays valid (the
                // invariant documented on `Texture2D::average_color`).
                srgb: true,
            },
        )
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        // White: the shader divides by the average, so a neutral stand-in
        // leaves the band color untouched while the real map streams in.
        let texture_data = TextureData::solid_color([255, 255, 255, 255]);
        Texture2D::init_from_data(
            texture_data,
            TextureOptions {
                compute_average: true,
                srgb: true,
                ..TextureOptions::default()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::AssetPipelineContext;

    // Corrupt/unknown bytes must become the fallback checkerboard, not a
    // panic (asset hot-reload can catch a file mid-write; the render thread
    // polls asset futures — a panic aborts the runtime).
    #[test]
    fn corrupt_png_falls_back_instead_of_panicking() {
        // Valid PNG magic, truncated body.
        let corrupt = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        let _ = TexturePipeline { srgb: true }.process(
            corrupt,
            &AssetCache::new(),
            AssetPipelineContext {},
        );
    }

    #[test]
    fn decodes_gif() {
        let mut gif = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 1, image::Rgba([10, 200, 30, 255]))
            .write_to(&mut gif, image::ImageFormat::Gif)
            .unwrap();
        let data = decode(gif.get_ref()).expect("decodes");
        assert_eq!((data.width, data.height), (2, 1));
        assert_eq!(&data.bytes[..4], &[10, 200, 30, 255]);
    }

    #[test]
    fn decodes_pcx() {
        // 1×1, index 0 through a palette whose first colour is (10, 200, 30).
        let mut pcx = vec![0u8; 128];
        pcx[..4].copy_from_slice(&[10, 5, 1, 8]);
        pcx[65] = 1;
        pcx[66] = 2;
        pcx.extend_from_slice(&[0, 0, 12, 10, 200, 30]);
        pcx.resize(pcx.len() + 765, 0);
        let data = decode(&pcx).expect("decodes");
        assert_eq!((data.width, data.height), (1, 1));
        assert_eq!(data.bytes, [10, 200, 30, 255]);
    }

    #[test]
    fn unknown_format_falls_back_instead_of_panicking() {
        let _ = TexturePipeline { srgb: true }.process(
            b"<html>not an image</html>".to_vec(),
            &AssetCache::new(),
            AssetPipelineContext {},
        );
    }
}
