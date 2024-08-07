use crate::{
    asset::{AssetCache, AssetPipeline},
    texture::{Texture2D, TextureData, TextureFormat, TextureOptions, PNG},
};

pub struct TexturePipeline;

impl AssetPipeline<Texture2D> for TexturePipeline {
    fn process(
        &self,
        bytes: Vec<u8>,
        _asset_cache: &AssetCache,
        _context: crate::asset::AssetPipelineContext,
    ) -> Texture2D {
        let texture_data = PNG.load(&bytes);
        Texture2D::init_from_data(texture_data, TextureOptions::default())
    }

    fn unloaded_asset(&self, _context: crate::asset::AssetPipelineContext) -> Texture2D {
        let texture_data = TextureData::checkerboard_pattern(8, 8, [0, 255, 0, 255]);
        Texture2D::init_from_data(texture_data, TextureOptions::default())
    }
}
