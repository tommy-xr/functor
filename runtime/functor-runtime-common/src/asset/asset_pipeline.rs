pub trait AssetPipeline<T> {
    fn materialize(&self, asset_path: &str, bytes: Vec<u8>) -> Result<T, String>;
}
