use std::{cell::RefCell, collections::HashMap, sync::Arc};

use super::{AssetCache, AssetHandle};

pub struct AssetPipelineContext {}

/// `Send + Sync`: a cache that decodes in the background runs `process` on
/// its worker threads.
pub trait AssetPipeline<TRuntimeAsset>: Send + Sync {
    fn process(
        &self,
        bytes: Vec<u8>,
        asset_cache: &AssetCache,
        context: AssetPipelineContext,
    ) -> TRuntimeAsset;

    fn unloaded_asset(&self, context: AssetPipelineContext) -> TRuntimeAsset;
}

pub fn build_pipeline<T>(pipeline: Box<dyn AssetPipeline<T>>) -> Arc<BuiltAssetPipeline<T>> {
    Arc::new(BuiltAssetPipeline {
        asset_pipeline: Arc::from(pipeline),
        asset_cache: RefCell::new(HashMap::new()),
    })
}

pub struct BuiltAssetPipeline<TRuntimeAsset> {
    asset_pipeline: Arc<dyn AssetPipeline<TRuntimeAsset>>,

    asset_cache: RefCell<HashMap<String, Arc<AssetHandle<TRuntimeAsset>>>>,
}

impl<TRuntimeAsset> BuiltAssetPipeline<TRuntimeAsset> {
    pub fn get_opt(&self, asset_name: &str) -> Option<Arc<AssetHandle<TRuntimeAsset>>> {
        self.asset_cache.borrow().get(asset_name).cloned()
    }

    pub fn cache(&self, asset_name: &str, asset: Arc<AssetHandle<TRuntimeAsset>>) {
        self.asset_cache
            .borrow_mut()
            .insert(asset_name.to_owned(), asset.clone());
    }

    /// Drop the cached handle for `asset_name` so the next lookup rebuilds it
    /// from (re-read) bytes — asset hot-reload. Scenes still holding the old
    /// handle keep rendering the old decode until their next draw resolves the
    /// path again (the very next frame, in practice).
    pub fn evict(&self, asset_name: &str) {
        self.asset_cache.borrow_mut().remove(asset_name);
    }
}

impl<TRuntimeAsset> BuiltAssetPipeline<TRuntimeAsset> {
    /// The pipeline itself, to run on any thread.
    pub(super) fn decoder(&self) -> Arc<dyn AssetPipeline<TRuntimeAsset>> {
        self.asset_pipeline.clone()
    }

    pub(super) fn unloaded_asset(&self, context: AssetPipelineContext) -> TRuntimeAsset {
        self.asset_pipeline.unloaded_asset(context)
    }
}
