use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use crate::io::{load_bytes_async, load_bytes_async2};

use super::{AssetHandle, AssetLoader, AssetPipeline, AssetPipelineContext};

pub struct AssetCache {
    bytes_cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl AssetCache {
    pub fn new() -> AssetCache {
        AssetCache {
            bytes_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn load_asset_with_pipeline<T: 'static>(
        &mut self,
        pipeline: Arc<dyn AssetPipeline<T> + 'static>,
        asset_path: &str,
    ) -> Arc<AssetHandle<T>> {
        let asset_path_owned = asset_path.to_string();
        let bytes_cache = self.bytes_cache.clone();

        // Check cache
        if let Some(cached_bytes) = self.bytes_cache.lock().unwrap().get(&asset_path_owned) {
            // TODO: Can we avoid the clone here
            let bytes = cached_bytes.clone();

            let pipeline = pipeline.clone();
            let context = super::AssetPipelineContext {};
            let default_asset = pipeline.unloaded_asset(context);

            let context = super::AssetPipelineContext {};
            let pipeline = pipeline.clone();
            // If bytes are already cached, return a handle immediately
            let future = async move { Ok(Arc::new(pipeline.process(bytes, context))) };
            return Arc::new(AssetHandle::new(future, Arc::new(default_asset)));
        }

        let context = AssetPipelineContext {};
        let pipeline = pipeline.clone();
        let default_asset = pipeline.unloaded_asset(context);

        // If not cached, load bytes asynchronously
        let bytes_future = async move {
            let bytes = load_bytes_async2(asset_path_owned.clone()).await?;
            bytes_cache
                .lock()
                .unwrap()
                .insert(asset_path_owned.clone(), bytes.clone());

            let pipeline = pipeline.clone();
            let context = AssetPipelineContext {};
            Ok(Arc::new(pipeline.process(bytes, context)))
        };

        let handle = Arc::new(AssetHandle::new(bytes_future, Arc::new(default_asset)));
        handle
    }
}
