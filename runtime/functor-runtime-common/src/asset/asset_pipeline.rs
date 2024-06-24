pub struct AssetPipelineContext {}

pub trait AssetPipeline<TRuntimeAsset> {
    fn materialize(&self, bytes: Vec<u8>, context: AssetPipelineContext) -> TRuntimeAsset;

    fn unloaded_asset(&self, context: AssetPipelineContext) -> TRuntimeAsset;
}
