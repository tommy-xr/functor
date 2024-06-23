use super::AssetLoader;

pub struct AssetCache {
    loader: Box<dyn AssetLoader>,
}

impl AssetCache {
    pub fn new(loader: Box<dyn AssetLoader>) -> AssetCache {
        AssetCache { loader }
    }
}
