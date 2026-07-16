use image::ImageFormat;

use crate::{
    asset::{AssetCache, AssetPipeline},
    texture::{Texture2D, TextureData, TextureFormat, TextureOptions, JPEG, PNG},
};

pub struct TexturePipeline;

impl AssetPipeline<Texture2D> for TexturePipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        context: crate::asset::AssetPipelineContext,
    ) -> Texture2D {
        // Unhandled formats and corrupt bytes both fall back to the
        // checkerboard, never a panic: asset hot-reload can catch a file
        // mid-write, and the render thread polls asset futures (a panic
        // aborts the runtime). The next save reloads it again.
        let decoded = match image::guess_format(&bytes) {
            Ok(ImageFormat::Png) => PNG.load(&bytes),
            Ok(ImageFormat::Jpeg) => JPEG.load(&bytes),
            other => Err(format!("unhandled format: {:?}", other)),
        };
        let texture_data = match decoded {
            Ok(data) => data,
            Err(e) => {
                eprintln!("[texture] cannot decode image: {e}");
                return self.unloaded_asset(context);
            }
        };
        Texture2D::init_from_data(
            texture_data,
            // Default to `wrap: true` so that model textures load correctly
            // TODO: Consider how to pass options to pipeline
            TextureOptions {
                wrap: true,
                linear: true,
            },
        )
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        let texture_data = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
        Texture2D::init_from_data(texture_data, TextureOptions::default())
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
