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
        _context: crate::asset::AssetPipelineContext,
    ) -> Texture2D {
        let guessed_format = image::guess_format(&bytes);

        let format_loader = match guessed_format {
            Ok(ImageFormat::Png) => PNG,
            Ok(ImageFormat::Jpeg) => JPEG,
            // TODO: Replace with placeholder texture
            _ => panic!("Unhandled format: {:?}", guessed_format),
        };

        let texture_data = format_loader.load(&bytes);
        Texture2D::init_from_data(texture_data, TextureOptions::default())
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        let texture_data = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
        Texture2D::init_from_data(texture_data, TextureOptions::default())
    }
}
