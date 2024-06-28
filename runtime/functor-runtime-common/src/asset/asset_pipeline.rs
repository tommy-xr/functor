use std::{cell::RefCell, collections::HashMap, sync::Arc};

use super::AssetHandle;

pub struct AssetPipelineContext {}

pub trait AssetPipeline<TRuntimeAsset> {
    fn process(&self, bytes: Vec<u8>, context: AssetPipelineContext) -> TRuntimeAsset;

    fn unloaded_asset(&self, context: AssetPipelineContext) -> TRuntimeAsset;
}

pub fn build<T>(pipeline: Box<dyn AssetPipeline<T>>) -> BuiltAssetPipeline<T> {
    BuiltAssetPipeline {
        asset_pipeline: pipeline,
        asset_cache: RefCell::new(HashMap::new()),
    }
}

pub struct BuiltAssetPipeline<TRuntimeAsset> {
    asset_pipeline: Box<dyn AssetPipeline<TRuntimeAsset>>,

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
}

impl<TRuntimeAsset> AssetPipeline<TRuntimeAsset> for BuiltAssetPipeline<TRuntimeAsset> {
    fn process(&self, bytes: Vec<u8>, context: AssetPipelineContext) -> TRuntimeAsset {
        self.asset_pipeline.process(bytes, context)
    }

    fn unloaded_asset(&self, context: AssetPipelineContext) -> TRuntimeAsset {
        self.asset_pipeline.unloaded_asset(context)
    }
}
