use crate::asset::AssetCache;

pub struct RenderContext<'a> {
    pub gl: &'a glow::Context,
    pub shader_version: &'a str,
    pub asset_cache: &'a AssetCache,
}
