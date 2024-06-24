use super::{AssetLoader, AssetPipeline};

pub struct AssetCache {
    loader: Box<dyn AssetLoader>,
}

pub struct AssetHandle<T> {
    pipeline: Box<dyn AssetPipeline<T> + 'static>,
}

impl<T> AssetHandle<T> {
    pub fn get(&self) -> T {
        unimplemented!()
    }
}

impl AssetCache {
    pub fn new(loader: Box<dyn AssetLoader>) -> AssetCache {
        AssetCache { loader }
    }

    pub fn load_asset_with_pipeline<T>(
        &mut self,
        pipeline: Box<dyn AssetPipeline<T> + 'static>,
        asset_path: &str,
    ) -> AssetHandle<T> {
        let handle = AssetHandle { pipeline };
        handle
    }
}
