use image::ImageFormat;

use crate::{
    asset::{AssetCache, AssetPipeline},
    texture::{
        Texture2D, TextureData, TextureFormat, TextureOptions, TERRAIN_DETAIL_ANISOTROPY, JPEG, PNG,
    },
};

/// Decode an image, or `None` if the bytes are not one we handle.
///
/// Unhandled formats and corrupt bytes both fall back to the checkerboard,
/// never a panic: asset hot-reload can catch a file mid-write, and the render
/// thread polls asset futures (a panic aborts the runtime). The next save
/// reloads it again.
fn decode(bytes: &Vec<u8>) -> Option<TextureData> {
    let decoded = match image::guess_format(bytes) {
        Ok(ImageFormat::Png) => PNG.load(bytes),
        Ok(ImageFormat::Jpeg) => JPEG.load(bytes),
        other => Err(format!("unhandled format: {:?}", other)),
    };
    match decoded {
        Ok(data) => Some(data),
        Err(e) => {
            eprintln!("[texture] cannot decode image: {e}");
            None
        }
    }
}

pub struct TexturePipeline;

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
                // moves. The fallback checkerboard below deliberately keeps
                // level 0 so a missing asset still reads as a flat marker.
                mipmap: true,
                // Albedo images are color: sRGB storage, hardware decode.
                srgb: true,
                ..TextureOptions::default()
            },
        )
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        let texture_data = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
        Texture2D::init_from_data(
            texture_data,
            TextureOptions {
                srgb: true,
                ..TextureOptions::default()
            },
        )
    }
}

/// Non-color ("data") textures — normal maps today. Same decode as
/// [`TexturePipeline`], but uploaded LINEAR: the texels are vectors, and an
/// sRGB decode would bend every normal (see `crate::color_space`).
pub struct LinearTexturePipeline;

impl AssetPipeline<Texture2D> for LinearTexturePipeline {
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
                ..TextureOptions::default()
            },
        )
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        // A flat "straight up" tangent-space normal: the identity stand-in
        // for a missing/undecodable normal map (a color checkerboard would
        // dent the surface it stands in for).
        let texture_data = TextureData::solid_color([128, 128, 255, 255]);
        Texture2D::init_from_data(texture_data, TextureOptions::default())
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
        let _ = TexturePipeline.process(corrupt, &AssetCache::new(), AssetPipelineContext {});
    }

    #[test]
    fn unknown_format_falls_back_instead_of_panicking() {
        let _ = TexturePipeline.process(
            b"<html>not an image</html>".to_vec(),
            &AssetCache::new(),
            AssetPipelineContext {},
        );
    }
}
