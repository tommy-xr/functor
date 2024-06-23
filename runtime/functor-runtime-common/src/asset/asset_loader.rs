use async_trait::async_trait;

#[async_trait]
pub trait AssetLoader {
    fn load_bytes(&self, path: &str) -> Result<Vec<u8>, String>;
}
