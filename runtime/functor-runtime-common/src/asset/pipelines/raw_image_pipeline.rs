use crate::{
    asset::{AssetCache, AssetPipeline},
    texture::{PixelFormat, TextureData},
};

/// Decode an image file to raw pixels (`TextureData`) with NO GL hydration —
/// for assets assembled from several files before a single GPU upload (a
/// cubemap's six faces), where per-file `Texture2D`s would be wasted 2D
/// textures. Decodes directly via the image crate (any format it recognizes,
/// no sprite color-keying); an unrecognized or corrupt file becomes a 0×0
/// sentinel the assembler treats as a failed face (`process` is infallible by
/// trait, and a bad asset must warn-and-disable, never panic the runtime).
pub struct RawImagePipeline;

impl AssetPipeline<TextureData> for RawImagePipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        _context: crate::asset::AssetPipelineContext,
    ) -> TextureData {
        match image::load_from_memory(&bytes) {
            Ok(image) => TextureData::from_image(image),
            Err(e) => {
                eprintln!("[raw-image] cannot decode image: {e}");
                TextureData {
                    bytes: vec![],
                    width: 0,
                    height: 0,
                    format: PixelFormat::RGBA,
                }
            }
        }
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> TextureData {
        TextureData::solid_color([128, 128, 128, 255])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::AssetPipelineContext;

    fn process(bytes: Vec<u8>) -> TextureData {
        RawImagePipeline.process(bytes, &AssetCache::new(), AssetPipelineContext {})
    }

    // A recognized-but-corrupt file must become the 0x0 sentinel, not a panic
    // (the render thread polls asset futures — a panic aborts the runtime).
    #[test]
    fn corrupt_image_becomes_the_sentinel() {
        // A valid PNG magic with a truncated body.
        let corrupt = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        let data = process(corrupt);
        assert_eq!((data.width, data.height), (0, 0));
    }

    #[test]
    fn unrecognized_bytes_become_the_sentinel() {
        let data = process(vec![1, 2, 3, 4]);
        assert_eq!((data.width, data.height), (0, 0));
    }
}
