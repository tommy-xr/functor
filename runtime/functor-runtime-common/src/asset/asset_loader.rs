use async_trait::async_trait;

#[async_trait]
pub trait AssetLoader: Sync + Send {
    async fn load_bytes(&self, path: &str) -> Result<Vec<u8>, String>;
}
