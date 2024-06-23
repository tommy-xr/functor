use super::{AssetLoader, AssetPipeline};

pub struct AssetCache {
    loader: Box<dyn AssetLoader>,
}

impl AssetCache {
    pub fn new(loader: Box<dyn AssetLoader>) -> AssetCache {
        AssetCache { loader }
    }

    pub fn load_asset<T>(&mut self, pipeline: Box<dyn AssetPipeline<T>>, asset_path: &str) -> T {
        unimplemented!("Need to implement")
    }
}
